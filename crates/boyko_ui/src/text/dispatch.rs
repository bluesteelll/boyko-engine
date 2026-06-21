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

use crate::components::{
    ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign, UiLayout, UiRoot,
    UiSpacing,
};
use crate::interaction::action::{OnClick, OnHover, OnSubmit};
use crate::text::ast::{CompKind, ParsedComponent};
use crate::text::report::UiParseReport;
use crate::text::split::split_top_level;
use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};

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
        // P4 action-emitting tuple newtypes carrying a dense `u16` action index
        // (Decision 3). The integer-index form `OnClick(3)` is reflection-free and
        // fully resolvable here. The action-NAME form `OnClick(Jump)` needs a
        // name→index table from the registered action enum, which is not threaded
        // into `parse_and_insert`; that form is a documented P4 deferral.
        "OnClick" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnClick must use the tuple form `OnClick(index)`");
                return Err(());
            }
            cmds.entity(entity).insert(OnClick(parse_action_index(body, body_col, "OnClick", rep)));
        }
        "OnHover" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnHover must use the tuple form `OnHover(index)`");
                return Err(());
            }
            cmds.entity(entity).insert(OnHover(parse_action_index(body, body_col, "OnHover", rep)));
        }
        "OnSubmit" => {
            if kind != CompKind::Tuple {
                rep.error(line_no, body_col, "OnSubmit must use the tuple form `OnSubmit(index)`");
                return Err(());
            }
            cmds.entity(entity)
                .insert(OnSubmit(parse_action_index(body, body_col, "OnSubmit", rep)));
        }
        // P4 data-bind components. RECOGNIZED here (so a well-formed `bind_text:`/
        // `bind_value:` is not misreported as an unknown component), but the `.ui`
        // form is a deferred P4 feature: it needs a name→Entity index for the
        // `source` widget AND a component-name→ComponentId + field-name→id resolver
        // (the `Bindable::field_id` table keyed by the source component's name),
        // neither of which is threaded into the spawn-time parser. The `ui!`
        // monomorphized path (which has the concrete source type in scope) and the
        // direct `BindText`/`BindValue` component inserts are the supported routes.
        // The architectural gap (threading name resolution into the `.ui` parser)
        // is escalated, not invented here. Mirrors the `OnClick(Jump)` action-name
        // deferral above.
        "BindText" | "BindValue" => {
            rep.error(
                line_no,
                body_col,
                format!(
                    "{name} is a deferred `.ui` feature: it requires source name \
                     resolution (a UiName→Entity index + a component/field-name \
                     resolver) not yet threaded into the spawn-time parser; use the \
                     `ui!` macro or insert {name} directly"
                ),
            );
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

/// Parses a dense `u16` action index for an `OnClick`/`OnHover`/`OnSubmit` tuple
/// (P4 Decision 3). On a parse failure (e.g. an action-NAME form, which is a
/// deferred P4 feature) it records a per-component error and returns
/// [`NO_ACTION`](crate::interaction::action::NO_ACTION) so the component inserts
/// with a no-op action rather than dropping the whole node.
fn parse_action_index(body: &str, body_col: u16, comp: &str, rep: &mut UiParseReport) -> u16 {
    match body.trim().parse::<u16>() {
        Ok(v) => v,
        Err(_) => {
            rep.error(
                line_of(rep),
                body_col,
                format!(
                    "{comp} expects a numeric action index `{comp}(n)`; \
                     the action-name form is a deferred P4 feature"
                ),
            );
            crate::interaction::action::NO_ACTION
        }
    }
}
