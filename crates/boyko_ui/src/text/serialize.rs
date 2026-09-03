//! The round-trip serializer: live UI subtree → canonical `.ui` text
//! (P3 §6, Decision 16).
//!
//! `parse_ui(serialize_ui(view))` is value/topology-equal to the source on
//! canonical input; `serialize → parse → serialize` is byte-identical. The
//! canonical field order + the pinned float rule + 4-space indent make the output
//! a normal form.
//!
//! # Byte-identity is NOT the anti-loss gate
//!
//! `serialize → parse → serialize` is byte-identical whenever the writer drops a
//! component CONSISTENTLY — the dropped data is absent from both texts, so it
//! never enters the comparison. That is how this writer emitted 8 of the 20
//! non-exempt members of its own parser's vocabulary under a green gate. Coverage
//! is enforced by two other mechanisms, not by that one:
//!
//! * COMPILE TIME — [`write_attached`] matches EXHAUSTIVELY on
//!   [`UiTextComponent`](crate::text::vocab::UiTextComponent), so a member added
//!   to the vocabulary is an `E0004` here until it is handled;
//! * RUN TIME — `p3_world_round_trip.rs` builds a world holding every emitted
//!   member with non-default values, serializes it, re-spawns the text into a
//!   SECOND world and compares the two WORLDS, which catches an arm that exists
//!   but writes nothing.
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

use boyko_ecs::ecs::core::entity::entity::Entity;

use crate::binding::components::{BindText, BindValue, TemplateId, NO_FIELD};
use crate::components::{
    AnchorEdge, ComputedClip, ComputedRect, ContentSize, UiAbsolute, UiAlign, UiAnchor, UiGrid,
    UiImage, UiLayout, UiSpacing,
};
use crate::reload::tree_view::{LiveNode, UiTreeView};
use crate::text::components::{TextAlign, UiText};
use crate::text::report::UI_FORMAT_VERSION;
use crate::text::vocab::UiTextComponent;
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

    // Attached components at depth+1, in the vocabulary's canonical order. The
    // roster is walked rather than hand-listed so a member added to
    // `UiTextComponent` cannot be forgotten here: `write_attached`'s match is
    // exhaustive, so the new variant is an `E0004` until it is handled.
    let attach_indent = indent + 4;
    for comp in UiTextComponent::ALL.iter().copied() {
        write_attached(view, node, comp, attach_indent, out);
    }

    // Children at depth+1 (their attached components land at depth+2).
    for &child in &node.children {
        if let Some(child_node) = view.get(child) {
            write_node(view, child_node, depth + 1, out);
        }
    }
}

/// Writes ONE attached-component line for `comp` if this node carries it, and
/// nothing otherwise.
///
/// The `match` is EXHAUSTIVE over the vocabulary on purpose (see
/// [`vocab`](crate::text::vocab)): it is the compile-time half of the
/// anti-loss gate. Exhaustiveness cannot see an arm that exists but writes
/// nothing, which is why `p3_world_round_trip.rs` re-reads the whole document out
/// of a second world — an arm that silently does nothing reds there.
fn write_attached(
    view: &UiTreeView,
    node: &LiveNode,
    comp: UiTextComponent,
    indent: u32,
    out: &mut String,
) {
    match comp {
        // Written on the node's HEAD line, above — never as an attached line.
        UiTextComponent::UiLayout => {}
        // Layout OUTPUT: a spawn-time seed `ui_layout_apply` overwrites, so writing
        // it back would pin a stale rect into the document (Decision 14 / §6).
        // `UiSourceOrder` is private and is not in the vocabulary at all.
        UiTextComponent::ComputedRect => {}

        UiTextComponent::UiSpacing => {
            if let Some(v) = node.spacing {
                line(out, indent, |o| write_ui_spacing(&v, o));
            }
        }
        UiTextComponent::UiAlign => {
            if let Some(v) = node.align {
                line(out, indent, |o| write_ui_align(&v, o));
            }
        }
        UiTextComponent::UiAbsolute => {
            if let Some(v) = node.absolute {
                line(out, indent, |o| write_ui_absolute(&v, o));
            }
        }
        UiTextComponent::ContentSize => {
            if let Some(v) = node.content_size {
                line(out, indent, |o| write_content_size(&v, o));
            }
        }
        UiTextComponent::StackIndex => {
            if let Some(v) = node.stack_index {
                line(out, indent, |o| {
                    let _ = write!(o, "StackIndex({})", v.0);
                });
            }
        }
        UiTextComponent::ComputedClip => {
            if let Some(v) = node.clip {
                line(out, indent, |o| write_computed_clip(&v, o));
            }
        }
        UiTextComponent::UiRoot => {
            if node.is_root {
                line(out, indent, |o| o.push_str("UiRoot"));
            }
        }
        UiTextComponent::UiText => {
            if let Some(v) = node.text {
                line(out, indent, |o| write_ui_text(&v, o));
            }
        }
        UiTextComponent::UiImage => {
            if let Some(v) = node.image {
                line(out, indent, |o| write_ui_image(&v, o));
            }
        }
        UiTextComponent::UiGrid => {
            if let Some(v) = node.grid {
                line(out, indent, |o| write_ui_grid(&v, o));
            }
        }
        UiTextComponent::UiAnchor => {
            if let Some(v) = node.anchor {
                line(out, indent, |o| write_ui_anchor(&v, o));
            }
        }
        UiTextComponent::Button => {
            if node.is_button {
                line(out, indent, |o| o.push_str("Button"));
            }
        }
        UiTextComponent::Bar => {
            if node.is_bar {
                line(out, indent, |o| o.push_str("Bar"));
            }
        }
        UiTextComponent::BarFill => {
            if node.is_bar_fill {
                line(out, indent, |o| o.push_str("BarFill"));
            }
        }
        UiTextComponent::OnClick => {
            if let Some(v) = node.on_click {
                line(out, indent, |o| {
                    let _ = write!(o, "OnClick({})", v.0);
                });
            }
        }
        UiTextComponent::OnHover => {
            if let Some(v) = node.on_hover {
                line(out, indent, |o| {
                    let _ = write!(o, "OnHover({})", v.0);
                });
            }
        }
        UiTextComponent::OnSubmit => {
            if let Some(v) = node.on_submit {
                line(out, indent, |o| {
                    let _ = write!(o, "OnSubmit({})", v.0);
                });
            }
        }
        UiTextComponent::BindText => {
            if let Some(v) = node.bind_text {
                line(out, indent, |o| write_bind_text(view, &v, o));
            }
        }
        UiTextComponent::BindValue => {
            if let Some(v) = node.bind_value {
                line(out, indent, |o| write_bind_value(view, &v, o));
            }
        }
    }
}

/// Writes one indented line: `indent` spaces, the body, a newline.
#[inline]
fn line(out: &mut String, indent: u32, body: impl FnOnce(&mut String)) {
    push_spaces(out, indent);
    body(out);
    out.push('\n');
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

fn write_ui_text(v: &UiText, out: &mut String) {
    out.push_str("UiText { color: ");
    let _ = write!(out, "{}", v.color);
    out.push_str(", size_px: ");
    write_f32_ui(v.size_px, out);
    // The dense font handle is written BARE (`font: 0`); the parser also accepts
    // the `FontId(0)` call form, and the bare form is the canonical one.
    out.push_str(", font: ");
    let _ = write!(out, "{}", v.font.0);
    out.push_str(", align: ");
    out.push_str(text_align_str(v.align));
    // `_pad` is not authorable and is never written.
    out.push_str(" }");
}

fn write_ui_image(v: &UiImage, out: &mut String) {
    out.push_str("UiImage { texture: ");
    let _ = write!(out, "{}", v.texture);
    out.push_str(", uv_min: ");
    write_f32_pair(v.uv_min, out);
    out.push_str(", uv_max: ");
    write_f32_pair(v.uv_max, out);
    out.push_str(", tint: ");
    let _ = write!(out, "{}", v.tint);
    out.push_str(" }");
}

fn write_ui_grid(v: &UiGrid, out: &mut String) {
    let _ = write!(out, "UiGrid {{ columns: {}, rows: {} }}", v.columns, v.rows);
}

fn write_ui_anchor(v: &UiAnchor, out: &mut String) {
    out.push_str("UiAnchor { edge: ");
    out.push_str(anchor_edge_str(v.edge));
    out.push_str(", offset_x: ");
    write_f32_ui(v.offset_x, out);
    out.push_str(", offset_y: ");
    write_f32_ui(v.offset_y, out);
    out.push_str(", use_safe_area: ");
    out.push_str(if v.use_safe_area { "true" } else { "false" });
    // `_pad` is not authorable and is never written.
    out.push_str(" }");
}

fn write_bind_text(view: &UiTreeView, v: &BindText, out: &mut String) {
    out.push_str("BindText { source: ");
    write_bind_source(view, v.source, out);
    let _ = write!(out, ", comp: {}, field: {}, field2: ", v.comp.0, v.field);
    write_field_opt(v.field2, out);
    out.push_str(", template: ");
    out.push_str(template_id_str(v.template));
    out.push_str(" }");
}

fn write_bind_value(view: &UiTreeView, v: &BindValue, out: &mut String) {
    out.push_str("BindValue { source: ");
    write_bind_source(view, v.source, out);
    let _ = write!(out, ", comp: {}, num_field: {}, den_field: ", v.comp.0, v.num_field);
    write_field_opt(v.den_field, out);
    out.push_str(" }");
}

/// Writes a bind `source`: the `#name` form when the target is a NAMED node of
/// this document, else the raw entity id.
///
/// The `#name` form is the only one that survives a reload, because it re-resolves
/// against the name index of the world the text is spawned into; a raw id is a
/// per-world handle. The numeric fallback is therefore a best effort for a source
/// OUTSIDE the document (a gameplay entity holding the bound component), and it
/// carries a known limitation: the parser reconstructs it with
/// `Entity::with_id`, i.e. GENERATION 0, so a bind to a non-zero-generation
/// entity reloads as a stale handle. Making that lossless needs a grammar that
/// can spell a generation — a format decision, not a writer one.
fn write_bind_source(view: &UiTreeView, source: Entity, out: &mut String) {
    match view.get(source).and_then(|n| n.name) {
        Some(name) => {
            out.push('#');
            out.push_str(name.as_str());
        }
        None => {
            let _ = write!(out, "{}", source.id().0);
        }
    }
}

/// Writes an optional field id: the `NO_FIELD` bareword for the sentinel (which
/// the parser accepts and which reads as intent), else the numeric id.
fn write_field_opt(field: u8, out: &mut String) {
    if field == NO_FIELD {
        out.push_str("NO_FIELD");
    } else {
        let _ = write!(out, "{field}");
    }
}

/// Writes a `[f32; 2]` in the parser's bracketed form (`[u, v]`).
fn write_f32_pair(v: [f32; 2], out: &mut String) {
    out.push('[');
    write_f32_ui(v[0], out);
    out.push_str(", ");
    write_f32_ui(v[1], out);
    out.push(']');
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

fn align_cross_str(a: AlignCross) -> &'static str {
    match a {
        AlignCross::Start => "Start",
        AlignCross::Center => "Center",
        AlignCross::End => "End",
        AlignCross::Stretch => "Stretch",
    }
}

fn text_align_str(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "Left",
        TextAlign::Center => "Center",
        TextAlign::Right => "Right",
    }
}

fn anchor_edge_str(e: AnchorEdge) -> &'static str {
    match e {
        AnchorEdge::TopLeft => "TopLeft",
        AnchorEdge::TopCenter => "TopCenter",
        AnchorEdge::TopRight => "TopRight",
        AnchorEdge::CenterLeft => "CenterLeft",
        AnchorEdge::Center => "Center",
        AnchorEdge::CenterRight => "CenterRight",
        AnchorEdge::BottomLeft => "BottomLeft",
        AnchorEdge::BottomCenter => "BottomCenter",
        AnchorEdge::BottomRight => "BottomRight",
    }
}

fn template_id_str(t: TemplateId) -> &'static str {
    match t {
        TemplateId::Value => "Value",
        TemplateId::Ratio => "Ratio",
    }
}

/// Appends `n` ASCII spaces.
#[inline]
fn push_spaces(out: &mut String, n: u32) {
    for _ in 0..n {
        out.push(' ');
    }
}
