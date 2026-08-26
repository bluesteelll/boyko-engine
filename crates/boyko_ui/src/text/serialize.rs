//! The round-trip serializer: live UI subtree → canonical `.ui` text
//! (P3 §6, Decision 16).
//!
//! `parse_ui(serialize_ui(view))` is value/topology-equal to the source on
//! canonical input; `serialize → parse → serialize` is byte-identical (the
//! gate). The canonical field order + the pinned float rule + 4-space indent
//! make the output a normal form.
//!
//! # The `.ui` float rule (Decision 16) — INVERSE of `write_f32`
//!
//! `boyko_input::persist::writer::write_f32` ALWAYS appends a `.0` to an
//! integral value (`0` → `0.0`). The `.ui` rule is the OPPOSITE: an integral
//! `f32` is emitted with NO decimal point (`320`), a fractional `f32` via Rust's
//! shortest round-trip `{}` (`0.15`). Both forms re-parse to bit-identical
//! `f32`s (Decision 16), and `Unit::Px(320.0)` (macro) == `Px(320)` (text) bits.
//! Do NOT "fix" the integral branch to append `.0` — that breaks the canonical
//! normal form and the round-trip gate.

use core::fmt::Write as _;

use crate::components::{
    ComputedClip, ComputedRect, ContentSize, NineSliceMode, SpriteAnimMode, UiAbsolute, UiAlign,
    UiLayout, UiNineSlice, UiSpacing, UiSpriteAnim, UiSpriteSheet,
};
use crate::reload::tree_view::{LiveNode, UiTreeView};
use crate::text::report::UI_FORMAT_VERSION;
use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};

/// Serialize a live UI subtree (rooted at the view's roots) back to canonical
/// `.ui` text appended to `out` (cleared first). Walks roots in snapshot
/// (entity-creation / pre-order) order.
pub fn serialize_ui(view: &UiTreeView, out: &mut String) {
    out.clear();
    out.push_str(
        "// boyko-engine .ui — generated; edits below the version line are canonicalized on rewrite\n",
    );
    let _ = writeln!(out, "version={UI_FORMAT_VERSION}");
    // Collect root entities in order, then recurse.
    let roots: Vec<_> = view.roots().map(|n| n.entity).collect();
    for root in roots {
        if let Some(node) = view.get(root) {
            write_node(view, node, 0, out);
        }
    }
}

/// Writes one node head + its attached components, then recurses children.
fn write_node(view: &UiTreeView, node: &LiveNode, depth: u32, out: &mut String) {
    let indent = depth * 4;
    push_spaces(out, indent);

    // Head line: `#name  UiLayout { .. }` (UiLayout always present — the macro
    // requires it; a node without one is skipped defensively).
    let Some(layout) = node.layout else {
        return;
    };
    if let Some(name) = &node.name {
        out.push('#');
        out.push_str(name.as_str());
        out.push_str("  ");
    }
    write_ui_layout(&layout, out);
    out.push('\n');

    // Attached components in a FIXED canonical order, each at depth+1.
    let attach_indent = indent + 4;
    if let Some(spacing) = node.spacing {
        push_spaces(out, attach_indent);
        write_ui_spacing(&spacing, out);
        out.push('\n');
    }
    if let Some(align) = node.align {
        push_spaces(out, attach_indent);
        write_ui_align(&align, out);
        out.push('\n');
    }
    if let Some(absolute) = node.absolute {
        push_spaces(out, attach_indent);
        write_ui_absolute(&absolute, out);
        out.push('\n');
    }
    if let Some(cs) = node.content_size {
        push_spaces(out, attach_indent);
        write_content_size(&cs, out);
        out.push('\n');
    }
    if let Some(si) = node.stack_index {
        push_spaces(out, attach_indent);
        let _ = write!(out, "StackIndex({})", si.0);
        out.push('\n');
    }
    if let Some(clip) = node.clip {
        push_spaces(out, attach_indent);
        write_computed_clip(&clip, out);
        out.push('\n');
    }
    // UI-ADVANCED S6 — the sprite vocabulary, appended AFTER the P1/P3 set so a
    // document carrying none of the three serializes byte-identically to what it
    // did before the rung. `UiSpriteCursor` is absent by construction: it is not
    // a `LiveNode` field, so an auto-inserted cursor is never written back and
    // the round trip is untouched by the `on_add` hook.
    if let Some(nine) = node.nine_slice {
        push_spaces(out, attach_indent);
        write_ui_nine_slice(&nine, out);
        out.push('\n');
    }
    if let Some(sheet) = node.sprite_sheet {
        push_spaces(out, attach_indent);
        write_ui_sprite_sheet(&sheet, out);
        out.push('\n');
    }
    if let Some(anim) = node.sprite_anim {
        push_spaces(out, attach_indent);
        write_ui_sprite_anim(&anim, out);
        out.push('\n');
    }
    if node.is_root {
        push_spaces(out, attach_indent);
        out.push_str("UiRoot\n");
    }
    // `ComputedRect` is layout output — OMITTED (Decision 14 / §6). It is a
    // spawn-time seed only; layout overwrites it. `UiSourceOrder` is private —
    // NEVER serialized.

    // Children at depth+1 (their attached components land at depth+2).
    for &child in &node.children {
        if let Some(child_node) = view.get(child) {
            write_node(view, child_node, depth + 1, out);
        }
    }
}

// ── Component writers (canonical field order) ─────────────────────────────────

fn write_ui_layout(v: &UiLayout, out: &mut String) {
    out.push_str("UiLayout { layout_type: ");
    out.push_str(layout_type_str(v.layout_type));
    out.push_str(", position_type: ");
    out.push_str(position_type_str(v.position_type));
    out.push_str(", width: ");
    write_unit(v.width, out);
    out.push_str(", height: ");
    write_unit(v.height, out);
    out.push_str(", min_width: ");
    write_unit(v.min_width, out);
    out.push_str(", min_height: ");
    write_unit(v.min_height, out);
    out.push_str(", max_width: ");
    write_unit(v.max_width, out);
    out.push_str(", max_height: ");
    write_unit(v.max_height, out);
    out.push_str(" }");
}

fn write_ui_spacing(v: &UiSpacing, out: &mut String) {
    out.push_str("UiSpacing { padding_left: ");
    write_unit(v.padding_left, out);
    out.push_str(", padding_right: ");
    write_unit(v.padding_right, out);
    out.push_str(", padding_top: ");
    write_unit(v.padding_top, out);
    out.push_str(", padding_bottom: ");
    write_unit(v.padding_bottom, out);
    out.push_str(", border_left: ");
    write_unit(v.border_left, out);
    out.push_str(", border_right: ");
    write_unit(v.border_right, out);
    out.push_str(", border_top: ");
    write_unit(v.border_top, out);
    out.push_str(", border_bottom: ");
    write_unit(v.border_bottom, out);
    out.push_str(", row_gap: ");
    write_unit(v.row_gap, out);
    out.push_str(", column_gap: ");
    write_unit(v.column_gap, out);
    out.push_str(" }");
}

fn write_ui_align(v: &UiAlign, out: &mut String) {
    out.push_str("UiAlign { main: ");
    out.push_str(align_main_str(v.main));
    out.push_str(", cross: ");
    out.push_str(align_cross_str(v.cross));
    out.push_str(" }");
}

fn write_ui_absolute(v: &UiAbsolute, out: &mut String) {
    out.push_str("UiAbsolute { left: ");
    write_unit(v.left, out);
    out.push_str(", right: ");
    write_unit(v.right, out);
    out.push_str(", top: ");
    write_unit(v.top, out);
    out.push_str(", bottom: ");
    write_unit(v.bottom, out);
    out.push_str(" }");
}

fn write_content_size(v: &ContentSize, out: &mut String) {
    out.push_str("ContentSize { width: ");
    write_f32_ui(v.width, out);
    out.push_str(", height: ");
    write_f32_ui(v.height, out);
    out.push_str(" }");
}

fn write_computed_clip(v: &ComputedClip, out: &mut String) {
    out.push_str("ComputedClip { x: ");
    write_f32_ui(v.x, out);
    out.push_str(", y: ");
    write_f32_ui(v.y, out);
    out.push_str(", w: ");
    write_f32_ui(v.w, out);
    out.push_str(", h: ");
    write_f32_ui(v.h, out);
    out.push_str(" }");
}

/// Writes a [`UiNineSlice`] (UI-ADVANCED S6). `_pad` is private and NOT written —
/// it is not authorable either, so the round trip is closed over the four
/// authored fields.
fn write_ui_nine_slice(v: &UiNineSlice, out: &mut String) {
    out.push_str("UiNineSlice { border_px: ");
    write_f32_quad(&v.border_px, out);
    out.push_str(", border_uv: ");
    write_f32_quad(&v.border_uv, out);
    out.push_str(", mode: ");
    out.push_str(nine_slice_mode_str(v.mode));
    out.push_str(", fill_center: ");
    out.push_str(if v.fill_center { "true" } else { "false" });
    out.push_str(" }");
}

/// Writes a [`UiSpriteSheet`] (UI-ADVANCED S6).
fn write_ui_sprite_sheet(v: &UiSpriteSheet, out: &mut String) {
    let _ = write!(out, "UiSpriteSheet {{ sheet: {}, index: {} }}", v.sheet, v.index);
}

/// Writes a [`UiSpriteAnim`] (UI-ADVANCED S6). `_pad` is private and NOT written.
fn write_ui_sprite_anim(v: &UiSpriteAnim, out: &mut String) {
    let _ = write!(out, "UiSpriteAnim {{ first: {}, last: {}, fps: ", v.first, v.last);
    write_f32_ui(v.fps, out);
    out.push_str(", mode: ");
    out.push_str(sprite_anim_mode_str(v.mode));
    let _ = write!(out, ", repeats: {} }}", v.repeats);
}

/// Writes a [`ComputedRect`] (only used by tests / completeness — the serializer
/// omits authored rects per §6, but the writer is provided for symmetry).
#[allow(dead_code)]
fn write_computed_rect(v: &ComputedRect, out: &mut String) {
    out.push_str("ComputedRect { x: ");
    write_f32_ui(v.x, out);
    out.push_str(", y: ");
    write_f32_ui(v.y, out);
    out.push_str(", w: ");
    write_f32_ui(v.w, out);
    out.push_str(", h: ");
    write_f32_ui(v.h, out);
    out.push_str(" }");
}

// ── Leaf formatters ──────────────────────────────────────────────────────────

fn write_unit(u: Unit, out: &mut String) {
    match u {
        Unit::Px(f) => {
            out.push_str("Px(");
            write_f32_ui(f, out);
            out.push(')');
        }
        Unit::Pct(f) => {
            out.push_str("Pct(");
            write_f32_ui(f, out);
            out.push(')');
        }
        Unit::Stretch(f) => {
            out.push_str("Stretch(");
            write_f32_ui(f, out);
            out.push(')');
        }
        Unit::Auto => out.push_str("Auto"),
    }
}

/// The `.ui`-specific float formatter (Decision 16): integral → NO decimal,
/// fractional → shortest round-trip `{}`. The INVERSE of `write_f32`'s `.0`
/// rule. Chosen so `parse_ui` re-reads bit-identical values.
///
/// Rust's `{}` for an integral `f32` prints no decimal point already (`320`,
/// `0`), and for a fractional value prints the shortest round-trip form
/// (`0.15`). So this is exactly the `{}` form with NO `.0` appended — the
/// inverse of `write_f32`. Do NOT append `.0` here.
fn write_f32_ui(v: f32, out: &mut String) {
    let _ = write!(out, "{v}");
}

fn layout_type_str(t: LayoutType) -> &'static str {
    match t {
        LayoutType::Row => "Row",
        LayoutType::Column => "Column",
        LayoutType::Overlay => "Overlay",
        LayoutType::Grid => "Grid",
    }
}

fn position_type_str(t: PositionType) -> &'static str {
    match t {
        PositionType::Relative => "Relative",
        PositionType::Absolute => "Absolute",
    }
}

fn align_main_str(a: AlignMain) -> &'static str {
    match a {
        AlignMain::Start => "Start",
        AlignMain::Center => "Center",
        AlignMain::End => "End",
        AlignMain::SpaceBetween => "SpaceBetween",
        AlignMain::SpaceAround => "SpaceAround",
        AlignMain::SpaceEvenly => "SpaceEvenly",
    }
}

/// Writes a `[f32; 4]` in the `.ui` bracketed form `[a, b, c, d]`, each component
/// through the `.ui` float rule — the inverse of [`parse_f32_quad`], so the four
/// values re-parse bit-identically.
///
/// [`parse_f32_quad`]: crate::text::dispatch
fn write_f32_quad(v: &[f32; 4], out: &mut String) {
    out.push('[');
    for (i, c) in v.iter().enumerate() {
        if i != 0 {
            out.push_str(", ");
        }
        write_f32_ui(*c, out);
    }
    out.push(']');
}

fn nine_slice_mode_str(m: NineSliceMode) -> &'static str {
    match m {
        NineSliceMode::Stretch => "Stretch",
        NineSliceMode::Tile => "Tile",
    }
}

fn sprite_anim_mode_str(m: SpriteAnimMode) -> &'static str {
    match m {
        SpriteAnimMode::Forward => "Forward",
        SpriteAnimMode::Reverse => "Reverse",
        SpriteAnimMode::PingPong => "PingPong",
        SpriteAnimMode::Once => "Once",
    }
}

fn align_cross_str(a: AlignCross) -> &'static str {
    match a {
        AlignCross::Start => "Start",
        AlignCross::Center => "Center",
        AlignCross::End => "End",
        AlignCross::Stretch => "Stretch",
    }
}

/// Appends `n` ASCII spaces.
#[inline]
fn push_spaces(out: &mut String, n: u32) {
    for _ in 0..n {
        out.push(' ');
    }
}
