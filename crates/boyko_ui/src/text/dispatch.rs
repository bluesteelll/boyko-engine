//! Reflection-free, type-directed component dispatch (P3 Decision 3 / 4 / 15).
//!
//! [`parse_and_insert`] is a single hand-written closed `match` over the
//! `boyko_ui` builtin component vocabulary (Pattern A): no serde, no
//! reflection, no `Any` / downcast / `TypeId`, no derive table. The match keys
//! on the component's TEXT name, which by invariant equals its Rust type name,
//! so a `.ui` file can ONLY construct UI components — a structural safety
//! property for untrusted/hand-edited text. An unknown name is a recoverable
//! per-line error.
//!
//! Per-field value parsing is TYPE-DIRECTED by the destination field
//! (Decision 4): there is NO standalone "parse a value" function. Each
//! `parse_<component>` starts from `T::default()` (the `..Default::default()`
//! semantics the `ui!` macro gives) and, for each `key: value` part, a
//! `match key` arm selects the leaf parser that statically knows the field's
//! type. This resolves the genuine token-shape collision — `Stretch(f)` is
//! `Unit::Stretch` ONLY in a `Unit` field and `AlignCross::Stretch` ONLY in an
//! `AlignCross` field; `Auto` is `Unit::Auto` only in a `Unit` field. A
//! cross-type literal is a recoverable per-field error, never a silent
//! mis-parse.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};
use boyko_input::resolve_action_name;

use crate::binding::components::{BindText, BindValue, TemplateId, NO_FIELD};
use crate::components::{
    AnchorEdge, Bar, BarFill, Button, ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute,
    UiAlign, UiAnchor, UiGrid, UiImage, UiLayout, UiName, UiRoot, UiSpacing,
};
use crate::interaction::action::{OnClick, OnHover, OnSubmit};
use crate::text::ast::{CompKind, ParsedComponent};
use crate::text::components::{FontId, TextAlign, UiText};
use crate::text::report::UiParseReport;
use crate::text::split::split_top_level;
use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};

/// The parsed-but-not-yet-sourced result of a `.ui` `BindText` / `BindValue`
/// whose `source` was authored as a `#name` forward/back reference (GUI #27).
///
/// Carries the fully-parsed component (its `source` field a placeholder the
/// resolver overwrites) plus the `#name` to resolve in pass 2. A NUMERIC
/// `source: N` does NOT produce one of these — it resolves at parse time and is
/// inserted in pass 1.
pub(crate) enum BindParse<C> {
    /// `source` was numeric — the component is final and inserts in pass 1.
    Resolved(C),
    /// `source` was `#name` — defer the whole insert to pass 2 (Decision: no
    /// sentinel; an unknown name simply never inserts and records an error).
    Deferred {
        /// The parsed component with every field but `source` final.
        comp: C,
        /// The `#name` to resolve to the source entity in pass 2.
        name: UiName,
        /// 1-based line of the bind component (for the unknown-name error).
        line_no: usize,
        /// 0-based body column (for the unknown-name error).
        body_col: u16,
    },
}

/// Dispatches a parsed component literal onto `ec`, parsing its body to the
/// typed component via the closed `match` (Decision 3) and inserting it.
///
/// `kind` distinguishes the struct / tuple / bare forms (Decision 15). Returns
/// `Ok(())` on a successful insert (per-field errors inside are recorded in
/// `rep` and the offending field keeps its `Default`); returns `Err(())` for a
/// hard per-component failure (unknown component, or a kind/component mismatch)
/// after recording the reason in `rep` — the caller drops only that component.
pub(crate) fn parse_and_insert(
    comp: &ParsedComponent,
    entity: Entity,
    cmds: &mut Commands,
    rep: &mut UiParseReport,
) -> Result<(), ()> {
    let name = comp.name.as_str();
    let body = comp.body.as_str();
    let kind = comp.kind;
    let line_no = comp.line_no;
    let body_col = comp.body_col;
    match name {
        "UiLayout" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_layout(body, body_col, rep));
        }
        "UiSpacing" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_spacing(body, body_col, rep));
        }
        "UiAlign" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_align(body, body_col, rep));
        }
        "UiAbsolute" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_absolute(body, body_col, rep));
        }
        "ContentSize" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_content_size(body, body_col, rep));
        }
        "UiText" => {
            // GUI P5b: the text STYLE component (content is the separate
            // `UiTextBuffer`, set via `#name`-bound data or a direct insert).
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_text(body, body_col, rep));
        }
        "ComputedRect" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_computed_rect(body, body_col, rep));
        }
        "ComputedClip" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_computed_clip(body, body_col, rep));
        }
        "StackIndex" => {
            // The ONLY P3 tuple newtype (Decision 15): `StackIndex(10)`.
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "StackIndex must use the tuple form `StackIndex(n)`");
                return Err(());
            }
            cmds.entity(entity).insert(parse_stack_index(body, body_col, rep));
        }
        "UiRoot" => {
            // A ZST marker: it carries no fields. A `UiRoot { ... }` / `UiRoot(x)`
            // is a recoverable error (the marker takes no body).
            if kind != CompKind::Bare {
                rep.error(line_no, body_col, "UiRoot is a marker and takes no fields");
                return Err(());
            }
            cmds.entity(entity).insert(UiRoot);
        }
        // GUI P6a widget markers — ZSTs, the `UiRoot` Bare precedent.
        "Button" => {
            if kind != CompKind::Bare {
                rep.error(line_no, body_col, "Button is a marker and takes no fields");
                return Err(());
            }
            cmds.entity(entity).insert(Button);
        }
        "Bar" => {
            if kind != CompKind::Bare {
                rep.error(line_no, body_col, "Bar is a marker and takes no fields");
                return Err(());
            }
            cmds.entity(entity).insert(Bar);
        }
        "BarFill" => {
            if kind != CompKind::Bare {
                rep.error(line_no, body_col, "BarFill is a marker and takes no fields");
                return Err(());
            }
            cmds.entity(entity).insert(BarFill);
        }
        // GUI P6a struct-form widget config/style components.
        "UiImage" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_image(body, body_col, rep));
        }
        "UiGrid" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_grid(body, body_col, rep));
        }
        "UiAnchor" => {
            expect_struct(name, kind, line_no, body_col, rep)?;
            cmds.entity(entity).insert(parse_ui_anchor(body, body_col, rep));
        }
        // Action-emitting tuple newtypes carrying a dense `u16` action index
        // (P4 Decision 3). BOTH forms resolve here (GUI #27): the integer-index
        // form `OnClick(3)` is reflection-free, and the action-NAME form
        // `OnClick(Jump)` resolves via the process-wide action-name table
        // (`boyko_input::resolve_action_name`, filled by `InputPlugin::build`).
        // A name with no registered enum / an unknown name records a recoverable
        // error and inserts `NO_ACTION` (the component still inserts; dispatch
        // fires nothing).
        "OnClick" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnClick must use the tuple form `OnClick(index)`");
                return Err(());
            }
            cmds.entity(entity)
                .insert(OnClick(parse_action_index(body, body_col, line_no, "OnClick", rep)));
        }
        "OnHover" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnHover must use the tuple form `OnHover(index)`");
                return Err(());
            }
            cmds.entity(entity)
                .insert(OnHover(parse_action_index(body, body_col, line_no, "OnHover", rep)));
        }
        "OnSubmit" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnSubmit must use the tuple form `OnSubmit(index)`");
                return Err(());
            }
            cmds.entity(entity)
                .insert(OnSubmit(parse_action_index(body, body_col, line_no, "OnSubmit", rep)));
        }
        // Data-bind components (P4 / GUI #27). These are NOT inserted here: their
        // `source` may be a `#name` forward-reference needing the two-pass resolve
        // (`lower_node` owns the fixup list + the name index). `lower_node` strips
        // `BindText` / `BindValue` from the generic insert loop and routes them
        // through `parse_bind_text` / `parse_bind_value`; reaching this arm means a
        // caller did not, which is an internal contract bug — record + drop, never
        // misreport as an unknown component.
        //
        // The `source` is the NAMED `#name` form (the GUI #27 LLM-authoring win)
        // or a numeric entity id; `comp` / `field` stay NUMERIC. The component-NAME
        // `comp: Health` / field-NAME `field: current` forms are a documented
        // followup (they need a type-erased `field_id` accessor in `boyko_macros`
        // + a universal name→ComponentId registry — both out of #27 scope).
        "BindText" | "BindValue" => {
            rep.error(line_no, body_col, format!("internal: {name} must be lowered via the bind fixup path"));
            return Err(());
        }
        // `UiName` is NOT dispatched here — it comes from the `#name` sigil only
        // (mirrors the macro, which inserts `UiName` from the binding name).
        other => {
            rep.error(line_no, body_col, format!("unknown component: {other:?}"));
            return Err(());
        }
    }
    Ok(())
}

/// Records a kind mismatch for a struct-form component invoked with the wrong
/// delimiter (e.g. `UiLayout(10)`), returning `Err(())`.
#[cold]
#[inline(never)]
fn kind_mismatch(name: &str, line_no: usize, body_col: u16, rep: &mut UiParseReport) {
    rep.error(line_no, body_col, format!("{name} must use the struct form `{name} {{ .. }}`"));
}

/// Asserts a component is the struct form; records + fails otherwise.
#[inline]
fn expect_struct(
    name: &str,
    kind: CompKind,
    line_no: usize,
    body_col: u16,
    rep: &mut UiParseReport,
) -> Result<(), ()> {
    if kind == CompKind::Struct {
        Ok(())
    } else {
        kind_mismatch(name, line_no, body_col, rep);
        Err(())
    }
}

/// Parses a `UiLayout` body for the spawn base (the lowering reads the typed
/// value directly rather than through `parse_and_insert`'s `ec.insert`).
#[inline]
pub(crate) fn parse_ui_layout_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> UiLayout {
    parse_ui_layout(body, body_col, rep)
}

/// Parses a `ComputedRect` body for the bundle fast-path spawn base.
#[inline]
pub(crate) fn parse_computed_rect_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> ComputedRect {
    parse_computed_rect(body, body_col, rep)
}

/// Parses a `UiSpacing` body (the reconcile patcher reads the typed value).
#[inline]
pub(crate) fn parse_ui_spacing_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> UiSpacing {
    parse_ui_spacing(body, body_col, rep)
}

/// Parses a `UiAlign` body (the reconcile patcher reads the typed value).
#[inline]
pub(crate) fn parse_ui_align_public(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiAlign {
    parse_ui_align(body, body_col, rep)
}

/// Parses a `UiAbsolute` body (the reconcile patcher reads the typed value).
#[inline]
pub(crate) fn parse_ui_absolute_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> UiAbsolute {
    parse_ui_absolute(body, body_col, rep)
}

/// Parses a `ContentSize` body (the reconcile patcher reads the typed value).
#[inline]
pub(crate) fn parse_content_size_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> ContentSize {
    parse_content_size(body, body_col, rep)
}

/// Parses a `ComputedClip` body (the reconcile patcher reads the typed value).
#[inline]
pub(crate) fn parse_computed_clip_public(
    body: &str,
    body_col: u16,
    rep: &mut UiParseReport,
) -> ComputedClip {
    parse_computed_clip(body, body_col, rep)
}

// ── Per-component parsers (default-then-overwrite, Decision 4) ────────────────

/// Iterates the top-level `key: value` fields of `body`, yielding
/// `(key, value, value_col)` with `value_col` the value's 0-based byte column in
/// the line. A field without a `:` separator is recorded as a per-field error
/// and skipped.
fn for_each_field<'a>(
    body: &'a str,
    body_col: u16,
    line_no: usize,
    rep: &mut UiParseReport,
    mut on_field: impl FnMut(&'a str, &'a str, u16, &mut UiParseReport),
) {
    // `split_top_level` borrows sub-slices of `body`; the byte offset of each
    // part within `body` is recovered by pointer arithmetic (the slices are
    // contiguous in `body`), giving a real column for field-error locality.
    let base = body.as_ptr() as usize;
    for part in split_top_level(body) {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            let off = (trimmed.as_ptr() as usize - base) as u16;
            rep.error(line_no, body_col.saturating_add(off), "expected `key: value`");
            continue;
        };
        let key = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        let value_off = (value.as_ptr() as usize - base) as u16;
        let value_col = body_col.saturating_add(value_off);
        on_field(key, value, value_col, rep);
    }
}

fn parse_ui_layout(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiLayout {
    let mut out = UiLayout::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "layout_type" => set(&mut out.layout_type, parse_layout_type(value), col, key, rep),
        "position_type" => set(&mut out.position_type, parse_position_type(value), col, key, rep),
        "width" => set(&mut out.width, parse_unit(value), col, key, rep),
        "height" => set(&mut out.height, parse_unit(value), col, key, rep),
        "min_width" => set(&mut out.min_width, parse_unit(value), col, key, rep),
        "min_height" => set(&mut out.min_height, parse_unit(value), col, key, rep),
        "max_width" => set(&mut out.max_width, parse_unit(value), col, key, rep),
        "max_height" => set(&mut out.max_height, parse_unit(value), col, key, rep),
        other => unknown_field("UiLayout", other, col, rep),
    });
    out
}

fn parse_ui_spacing(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiSpacing {
    let mut out = UiSpacing::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "padding_left" => set(&mut out.padding_left, parse_unit(value), col, key, rep),
        "padding_right" => set(&mut out.padding_right, parse_unit(value), col, key, rep),
        "padding_top" => set(&mut out.padding_top, parse_unit(value), col, key, rep),
        "padding_bottom" => set(&mut out.padding_bottom, parse_unit(value), col, key, rep),
        "border_left" => set(&mut out.border_left, parse_unit(value), col, key, rep),
        "border_right" => set(&mut out.border_right, parse_unit(value), col, key, rep),
        "border_top" => set(&mut out.border_top, parse_unit(value), col, key, rep),
        "border_bottom" => set(&mut out.border_bottom, parse_unit(value), col, key, rep),
        "row_gap" => set(&mut out.row_gap, parse_unit(value), col, key, rep),
        "column_gap" => set(&mut out.column_gap, parse_unit(value), col, key, rep),
        other => unknown_field("UiSpacing", other, col, rep),
    });
    out
}

fn parse_ui_align(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiAlign {
    let mut out = UiAlign::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "main" => set(&mut out.main, parse_align_main(value), col, key, rep),
        "cross" => set(&mut out.cross, parse_align_cross(value), col, key, rep),
        other => unknown_field("UiAlign", other, col, rep),
    });
    out
}

fn parse_ui_absolute(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiAbsolute {
    let mut out = UiAbsolute::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "left" => set(&mut out.left, parse_unit(value), col, key, rep),
        "right" => set(&mut out.right, parse_unit(value), col, key, rep),
        "top" => set(&mut out.top, parse_unit(value), col, key, rep),
        "bottom" => set(&mut out.bottom, parse_unit(value), col, key, rep),
        other => unknown_field("UiAbsolute", other, col, rep),
    });
    out
}

fn parse_content_size(body: &str, body_col: u16, rep: &mut UiParseReport) -> ContentSize {
    let mut out = ContentSize::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "width" => set(&mut out.width, parse_f32(value), col, key, rep),
        "height" => set(&mut out.height, parse_f32(value), col, key, rep),
        other => unknown_field("ContentSize", other, col, rep),
    });
    out
}

/// Parses a `UiText` body (GUI P5b): `color` (RGBA8 `u32`), `size_px` (`f32`), `font`
/// (a dense `u16` index), `align` (`Left`/`Center`/`Right`). Default-then-overwrite.
fn parse_ui_text(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiText {
    let mut out = UiText::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "color" => set(&mut out.color, parse_u32(value), col, key, rep),
        "size_px" => set(&mut out.size_px, parse_f32(value), col, key, rep),
        "font" => set(&mut out.font, parse_font_id(value), col, key, rep),
        "align" => set(&mut out.align, parse_text_align(value), col, key, rep),
        other => unknown_field("UiText", other, col, rep),
    });
    out
}

/// Parses a `UiImage` body (GUI P6a): `texture` (dense `u32` handle), `uv_min`/
/// `uv_max` (`[f32; 2]` as `[u, v]`), `tint` (RGBA8 `u32`). Default-then-overwrite.
fn parse_ui_image(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiImage {
    let mut out = UiImage::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "texture" => set(&mut out.texture, parse_u32(value), col, key, rep),
        "uv_min" => set(&mut out.uv_min, parse_f32_pair(value), col, key, rep),
        "uv_max" => set(&mut out.uv_max, parse_f32_pair(value), col, key, rep),
        "tint" => set(&mut out.tint, parse_u32(value), col, key, rep),
        other => unknown_field("UiImage", other, col, rep),
    });
    out
}

/// Parses a `UiGrid` body (GUI P6a): `columns`/`rows` (`u8` track counts).
/// Default-then-overwrite.
fn parse_ui_grid(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiGrid {
    let mut out = UiGrid::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "columns" => set(&mut out.columns, parse_u8(value), col, key, rep),
        "rows" => set(&mut out.rows, parse_u8(value), col, key, rep),
        other => unknown_field("UiGrid", other, col, rep),
    });
    out
}

/// Parses a `UiAnchor` body (GUI P6a): `edge` ([`AnchorEdge`]), `offset_x`/
/// `offset_y` (`f32`), `use_safe_area` (`bool`). The private `_pad` is not
/// authorable. Default-then-overwrite.
fn parse_ui_anchor(body: &str, body_col: u16, rep: &mut UiParseReport) -> UiAnchor {
    let mut out = UiAnchor::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "edge" => set(&mut out.edge, parse_anchor_edge(value), col, key, rep),
        "offset_x" => set(&mut out.offset_x, parse_f32(value), col, key, rep),
        "offset_y" => set(&mut out.offset_y, parse_f32(value), col, key, rep),
        "use_safe_area" => set(&mut out.use_safe_area, parse_bool(value), col, key, rep),
        other => unknown_field("UiAnchor", other, col, rep),
    });
    out
}

fn parse_computed_rect(body: &str, body_col: u16, rep: &mut UiParseReport) -> ComputedRect {
    let mut out = ComputedRect::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "x" => set(&mut out.x, parse_f32(value), col, key, rep),
        "y" => set(&mut out.y, parse_f32(value), col, key, rep),
        "w" => set(&mut out.w, parse_f32(value), col, key, rep),
        "h" => set(&mut out.h, parse_f32(value), col, key, rep),
        other => unknown_field("ComputedRect", other, col, rep),
    });
    out
}

fn parse_computed_clip(body: &str, body_col: u16, rep: &mut UiParseReport) -> ComputedClip {
    let mut out = ComputedClip::default();
    for_each_field(body, body_col, line_of(rep), rep, |key, value, col, rep| match key {
        "x" => set(&mut out.x, parse_f32(value), col, key, rep),
        "y" => set(&mut out.y, parse_f32(value), col, key, rep),
        "w" => set(&mut out.w, parse_f32(value), col, key, rep),
        "h" => set(&mut out.h, parse_f32(value), col, key, rep),
        other => unknown_field("ComputedClip", other, col, rep),
    });
    out
}

fn parse_stack_index(body: &str, body_col: u16, rep: &mut UiParseReport) -> StackIndex {
    match parse_u32(body.trim()) {
        Some(v) => StackIndex(v),
        None => {
            rep.error(line_of(rep), body_col, format!("invalid StackIndex value: {:?}", body.trim()));
            StackIndex::default()
        }
    }
}

// ── Field-write helper + leaf parsers (type-directed, Decision 4) ─────────────

/// Writes `parsed` into `dst` if it parsed, else records a per-field error.
#[inline]
fn set<T>(
    dst: &mut T,
    parsed: Option<T>,
    col: u16,
    key: &str,
    rep: &mut UiParseReport,
) {
    match parsed {
        Some(v) => *dst = v,
        None => rep.error(line_of(rep), col, format!("invalid value for field `{key}`")),
    }
}

/// Records an unknown field name on a component.
#[cold]
#[inline(never)]
fn unknown_field(comp: &str, field: &str, col: u16, rep: &mut UiParseReport) {
    rep.error(line_of(rep), col, format!("unknown field `{field}` on {comp}"));
}

/// The current line for a field-error. `for_each_field` is keyed on a single
/// component line, recorded into the report's transient line slot via
/// `with_line` so the per-field helpers do not need to thread it explicitly.
#[inline]
fn line_of(rep: &UiParseReport) -> usize {
    rep.current_line()
}

/// Parses a [`Unit`] value (the `Unit` field arm): `Px(f)`, `Pct(f)`,
/// `Stretch(f)`, or the bare `Auto` (Decision 4). An enum-style ident here (a
/// bare `Stretch`, or `Center`) is rejected — `None` → a per-field error.
fn parse_unit(value: &str) -> Option<Unit> {
    let v = value.trim();
    if v == "Auto" || v == "Unit::Auto" {
        return Some(Unit::Auto);
    }
    let f = |inner: &str| parse_f32(inner.trim());
    if let Some(rest) = strip_call(v, "Px").or_else(|| strip_call(v, "Unit::Px")) {
        return f(rest).map(Unit::Px);
    }
    if let Some(rest) = strip_call(v, "Pct").or_else(|| strip_call(v, "Unit::Pct")) {
        return f(rest).map(Unit::Pct);
    }
    if let Some(rest) = strip_call(v, "Stretch").or_else(|| strip_call(v, "Unit::Stretch")) {
        return f(rest).map(Unit::Stretch);
    }
    None
}

/// Strips `name(` … `)` returning the inner span, or `None` if `v` is not that
/// call shape. Tolerates an optional space before `(`.
fn strip_call<'a>(v: &'a str, name: &str) -> Option<&'a str> {
    let rest = v.strip_prefix(name)?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner)
}

/// Parses a [`LayoutType`] (bare or `LayoutType::`-qualified) — the
/// `LayoutType` field arm.
fn parse_layout_type(value: &str) -> Option<LayoutType> {
    match strip_qualifier(value.trim(), "LayoutType") {
        "Row" => Some(LayoutType::Row),
        "Column" => Some(LayoutType::Column),
        "Overlay" => Some(LayoutType::Overlay),
        "Grid" => Some(LayoutType::Grid),
        _ => None,
    }
}

/// Parses a [`PositionType`] (bare or `PositionType::`-qualified).
fn parse_position_type(value: &str) -> Option<PositionType> {
    match strip_qualifier(value.trim(), "PositionType") {
        "Relative" => Some(PositionType::Relative),
        "Absolute" => Some(PositionType::Absolute),
        _ => None,
    }
}

/// Parses an [`AlignMain`] (bare or `AlignMain::`-qualified).
fn parse_align_main(value: &str) -> Option<AlignMain> {
    match strip_qualifier(value.trim(), "AlignMain") {
        "Start" => Some(AlignMain::Start),
        "Center" => Some(AlignMain::Center),
        "End" => Some(AlignMain::End),
        "SpaceBetween" => Some(AlignMain::SpaceBetween),
        "SpaceAround" => Some(AlignMain::SpaceAround),
        "SpaceEvenly" => Some(AlignMain::SpaceEvenly),
        _ => None,
    }
}

/// Parses an [`AlignCross`] (bare or `AlignCross::`-qualified) — the
/// `AlignCross` field arm. `Stretch` here is `AlignCross::Stretch`, NOT
/// `Unit::Stretch` (Decision 4); a `Px(..)`/`Auto` here is rejected.
fn parse_align_cross(value: &str) -> Option<AlignCross> {
    match strip_qualifier(value.trim(), "AlignCross") {
        "Start" => Some(AlignCross::Start),
        "Center" => Some(AlignCross::Center),
        "End" => Some(AlignCross::End),
        "Stretch" => Some(AlignCross::Stretch),
        _ => None,
    }
}

/// Strips an optional `Type::` qualifier prefix from an enum-variant token.
#[inline]
fn strip_qualifier<'a>(value: &'a str, ty: &str) -> &'a str {
    // Accept both `Row` and `LayoutType::Row`.
    if let Some(rest) = value.strip_prefix(ty)
        && let Some(variant) = rest.strip_prefix("::")
    {
        return variant;
    }
    value
}

/// Parses an `f32` (the `f32` field arm). Accepts integral (`0`, `320`) and
/// fractional (`0.15`) forms — both round-trip through the canonical serializer
/// (Decision 16).
#[inline]
fn parse_f32(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok()
}

/// Parses a `u32` (the `u32` field arm).
#[inline]
fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

/// Parses a `u8` (the `u8` field arm — GUI P6a `UiGrid` track counts).
#[inline]
fn parse_u8(value: &str) -> Option<u8> {
    value.trim().parse::<u8>().ok()
}

/// Parses a `bool` (the `bool` field arm — GUI P6a `UiAnchor::use_safe_area`).
#[inline]
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parses a `[f32; 2]` (the `[f32; 2]` field arm — GUI P6a `UiImage` UVs). Accepts
/// the bracketed form `[u, v]`; the two comma-separated parts each parse as `f32`.
fn parse_f32_pair(value: &str) -> Option<[f32; 2]> {
    let v = value.trim();
    let inner = v.strip_prefix('[')?.strip_suffix(']')?;
    let mut it = inner.split(',');
    let a = parse_f32(it.next()?)?;
    let b = parse_f32(it.next()?)?;
    if it.next().is_some() {
        return None; // more than two components
    }
    Some([a, b])
}

/// Parses an [`AnchorEdge`] (bare or `AnchorEdge::`-qualified — GUI P6a).
fn parse_anchor_edge(value: &str) -> Option<AnchorEdge> {
    match strip_qualifier(value.trim(), "AnchorEdge") {
        "TopLeft" => Some(AnchorEdge::TopLeft),
        "TopCenter" => Some(AnchorEdge::TopCenter),
        "TopRight" => Some(AnchorEdge::TopRight),
        "CenterLeft" => Some(AnchorEdge::CenterLeft),
        "Center" => Some(AnchorEdge::Center),
        "CenterRight" => Some(AnchorEdge::CenterRight),
        "BottomLeft" => Some(AnchorEdge::BottomLeft),
        "BottomCenter" => Some(AnchorEdge::BottomCenter),
        "BottomRight" => Some(AnchorEdge::BottomRight),
        _ => None,
    }
}

/// Parses a dense [`FontId`] (the `FontId` field arm, GUI P5b): a bare `u16` index, or
/// the tuple form `FontId(0)`.
#[inline]
fn parse_font_id(value: &str) -> Option<FontId> {
    let v = value.trim();
    let inner = strip_call(v, "FontId").unwrap_or(v).trim();
    inner.parse::<u16>().ok().map(FontId)
}

/// Parses a [`TextAlign`] (bare or `TextAlign::`-qualified — GUI P5b).
#[inline]
fn parse_text_align(value: &str) -> Option<TextAlign> {
    match strip_qualifier(value.trim(), "TextAlign") {
        "Left" => Some(TextAlign::Left),
        "Center" => Some(TextAlign::Center),
        "Right" => Some(TextAlign::Right),
        _ => None,
    }
}

/// Parses a dense `u16` action index for an `OnClick`/`OnHover`/`OnSubmit` tuple
/// (P4 Decision 3 + GUI #27). Numeric-first: `OnClick(3)` parses the literal.
/// Otherwise the body is treated as an action NAME and resolved via the
/// process-wide action-name table ([`resolve_action_name`], filled by
/// `InputPlugin::build`); `OnClick(Jump)` lowers to the SAME index as the numeric
/// / `ui!` form (the equivalence gate).
///
/// On an unknown name (or no registered enum) it records a recoverable per-line
/// error and returns [`NO_ACTION`](crate::interaction::action::NO_ACTION) so the
/// component inserts with a no-op action rather than dropping the whole node.
fn parse_action_index(
    body: &str,
    body_col: u16,
    line_no: usize,
    comp: &str,
    rep: &mut UiParseReport,
) -> u16 {
    let token = body.trim();
    if let Ok(v) = token.parse::<u16>() {
        return v;
    }
    match resolve_action_name(token) {
        Some(idx) => idx,
        None => {
            rep.error(
                line_no,
                body_col,
                format!("{comp}: unknown action name {token:?} (not a registered action or a numeric index)"),
            );
            crate::interaction::action::NO_ACTION
        }
    }
}

// ── GUI #27: data-bind parsers (named `#name` source + numeric comp/field) ────

/// The placeholder source for a `BindParse::Deferred` component before pass-2
/// resolution. Never inserted: a deferred component is inserted ONLY after its
/// `#name` resolves (Decision: defer the whole insert, no sentinel reaches the
/// world). Entity id 0 is fine here since the field is overwritten before insert.
#[inline]
fn placeholder_source() -> Entity {
    Entity::with_id(EntityId(0))
}

/// Parses a `BindText` body (GUI #27): `source` (a `#name` ref OR a numeric entity
/// id), `comp` (numeric `ComponentId`), `field`/`field2` (`u8`, `NO_FIELD`/`255`
/// for an unused `field2`), `template` (`Value`/`Ratio`). Default-then-overwrite.
///
/// A `#name` `source` returns [`BindParse::Deferred`] (the whole insert is
/// deferred to the caller's pass-2 resolve); a numeric `source` returns
/// [`BindParse::Resolved`] and inserts in pass 1.
pub(crate) fn parse_bind_text(
    body: &str,
    body_col: u16,
    line_no: usize,
    rep: &mut UiParseReport,
) -> BindParse<BindText> {
    let mut out = BindText {
        source: placeholder_source(),
        comp: ComponentId(0),
        field: 0,
        field2: NO_FIELD,
        template: TemplateId::default(),
    };
    let mut deferred_name: Option<UiName> = None;
    for_each_field(body, body_col, line_no, rep, |key, value, col, rep| match key {
        "source" => set_source(value, col, &mut out.source, &mut deferred_name, rep),
        "comp" => set(&mut out.comp, parse_component_id(value), col, key, rep),
        "field" => set(&mut out.field, parse_u8(value), col, key, rep),
        "field2" => set(&mut out.field2, parse_field_opt(value), col, key, rep),
        "template" => set(&mut out.template, parse_template_id(value), col, key, rep),
        other => unknown_field("BindText", other, col, rep),
    });
    match deferred_name {
        Some(name) => BindParse::Deferred { comp: out, name, line_no, body_col },
        None => BindParse::Resolved(out),
    }
}

/// Parses a `BindValue` body (GUI #27): `source` (a `#name` ref OR a numeric
/// entity id), `comp` (numeric `ComponentId`), `num_field` (`u8`), `den_field`
/// (`u8`, `NO_FIELD`/`255` for a raw value). Default-then-overwrite.
pub(crate) fn parse_bind_value(
    body: &str,
    body_col: u16,
    line_no: usize,
    rep: &mut UiParseReport,
) -> BindParse<BindValue> {
    let mut out = BindValue {
        source: placeholder_source(),
        comp: ComponentId(0),
        num_field: 0,
        den_field: NO_FIELD,
    };
    let mut deferred_name: Option<UiName> = None;
    for_each_field(body, body_col, line_no, rep, |key, value, col, rep| match key {
        "source" => set_source(value, col, &mut out.source, &mut deferred_name, rep),
        "comp" => set(&mut out.comp, parse_component_id(value), col, key, rep),
        "num_field" => set(&mut out.num_field, parse_u8(value), col, key, rep),
        "den_field" => set(&mut out.den_field, parse_field_opt(value), col, key, rep),
        other => unknown_field("BindValue", other, col, rep),
    });
    match deferred_name {
        Some(name) => BindParse::Deferred { comp: out, name, line_no, body_col },
        None => BindParse::Resolved(out),
    }
}

/// Parses the `source` field: a `#name` reference (deferred to pass 2) or a
/// numeric entity id (resolved in place). A `#name` records the [`UiName`] into
/// `deferred_name`; a numeric id writes `dst`. A malformed value records a
/// per-field error and leaves the placeholder (the caller treats a still-deferred
/// `None` + placeholder as a normal numeric default).
fn set_source(
    value: &str,
    col: u16,
    dst: &mut Entity,
    deferred_name: &mut Option<UiName>,
    rep: &mut UiParseReport,
) {
    let v = value.trim();
    if let Some(name) = v.strip_prefix('#') {
        let name = name.trim();
        if name.is_empty() || name.len() > UiName::CAP {
            rep.error(line_of(rep), col, "invalid `#name` source (empty or too long)");
            return;
        }
        *deferred_name = Some(UiName::new(name));
        return;
    }
    match v.parse::<usize>() {
        Ok(id) => *dst = Entity::with_id(EntityId(id)),
        Err(_) => rep.error(line_of(rep), col, "invalid `source` (expected `#name` or a numeric entity id)"),
    }
}

/// Parses a numeric [`ComponentId`] (the `comp` field arm). `ComponentId` is a
/// `usize` newtype, so the value parses as `usize` and wraps.
#[inline]
fn parse_component_id(value: &str) -> Option<ComponentId> {
    value.trim().parse::<usize>().ok().map(ComponentId)
}

/// Parses an optional field id (`field2` / `den_field`): a `u8`, or the bareword
/// `NO_FIELD` — both map to [`NO_FIELD`]. A bare `255` also yields `NO_FIELD`.
#[inline]
fn parse_field_opt(value: &str) -> Option<u8> {
    let v = value.trim();
    if v == "NO_FIELD" {
        return Some(NO_FIELD);
    }
    v.parse::<u8>().ok()
}

/// Parses a [`TemplateId`] (bare or `TemplateId::`-qualified — GUI P4 / #27):
/// `Value` (`"{0}"`) or `Ratio` (`"{0}/{1}"`).
#[inline]
fn parse_template_id(value: &str) -> Option<TemplateId> {
    match strip_qualifier(value.trim(), "TemplateId") {
        "Value" => Some(TemplateId::Value),
        "Ratio" => Some(TemplateId::Ratio),
        _ => None,
    }
}
