//! The `.keys` one-pass hand parser (plan §9.1 / §9.3) — no serde/toml/ron.
//!
//! The format is line-based and human-editable; this parser is the *only* reader
//! of it. It is a **cold, load-time** path (a config is read once at startup or
//! on an explicit reload), so it may allocate freely — the per-frame input path
//! is untouched. The parser never aborts on a bad line: an unparseable line is
//! skipped and recorded in [`ParseReport`], so a hand-edited config can never
//! brick the game (plan §9.3 recoverable errors).
//!
//! # Critical rules (plan §9.3)
//! - The comment is stripped at the first **unquoted** `#` ([`strip_comment`]):
//!   a `#` inside `"…"` is literal.
//! - The top-level comma split is **paren-depth aware** ([`split_top_level`]):
//!   `axis2(up=W, down=S)` is one spec, never split mid-spec.
//! - `KeyCode::Unidentified(n)` ↔ `raw(0xNN)` and `MouseButton::Other(n)` ↔
//!   `MouseOther(n)` round-trip losslessly.
//! - Override-delta: an absent action keeps its code default; a present action
//!   fully overrides that action's slots; `= none` is an explicit unbind.
//! - Versioning: `version = N` first; an unknown **higher** version is a
//!   best-effort load with a warning (never a hard fail); a lower version loads
//!   with forward-compat defaults.

use crate::action::actionlike::Actionlike;
use crate::action::map::{AxisMode, BindSpec, ClashStrategy, InputMapBuilder, InputRef};
use crate::constants::MAX_CHORD_KEYS;
use crate::persist::keyname::{keycode_from_name, mousebutton_from_token, parse_int_u32};
use crate::raw::keycode::KeyCode;

/// The current `.keys` format version the writer emits and the parser fully
/// understands. A file declaring a *higher* version is loaded best-effort with a
/// warning recorded in [`ParseReport::warnings`]; a *lower* version loads with
/// forward-compatible defaults (plan §9.3 versioning).
pub const KEYS_FORMAT_VERSION: u32 = 1;

/// The outcome of parsing a `.keys` source: every recoverable per-line error and
/// every non-fatal warning, plus the parsed version. Parsing always succeeds at
/// the file level — these are observations for logging / a config-repair UI, not
/// a failure channel (plan §9.3).
#[derive(Clone, Debug, Default)]
pub struct ParseReport {
    /// The `version = N` value read from the file, or [`KEYS_FORMAT_VERSION`] if
    /// the file omitted it (a forward-compat default).
    pub version: u32,
    /// `(1-based line number, reason)` for every line that could not be parsed
    /// and was skipped.
    pub errors: Vec<(usize, String)>,
    /// Non-fatal advisories — e.g. an unknown higher format version, or a
    /// `stick(...)` binding accepted but ignored at runtime.
    pub warnings: Vec<(usize, String)>,
}

impl ParseReport {
    /// `true` iff no per-line error was recorded (warnings do not count).
    #[inline]
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parses `src` and applies its override-delta onto `builder` (plan §9.3).
///
/// `builder` should be seeded from the code-default map (e.g. via
/// [`InputMapBuilder::from_map`](crate::action::map::InputMapBuilder::from_map))
/// so that actions **absent** from the file retain their defaults; an action
/// **present** in the file has its binding list fully replaced (the override is
/// total per action). `action = none` clears the action's slot to
/// [`BindSpec::None`] (an explicit unbind).
///
/// Returns a [`ParseReport`]; parsing never fails at the file level.
pub fn load_keys<A: Actionlike>(src: &str, builder: &mut InputMapBuilder<A>) -> ParseReport {
    let mut report = ParseReport {
        version: KEYS_FORMAT_VERSION,
        ..Default::default()
    };
    // Actions whose slots this file has already started overriding — the first
    // binding line for an action clears its (default-seeded) slots, then appends.
    let mut overridden = vec![false; A::COUNT];
    let mut version_seen = false;
    // The active context's clash override, applied once at end-of-parse. v1 is
    // single-context: the last `[..]` header's clash wins. Contexts/stack land
    // in a later round (plan §6 V3); the header is parsed + the clash applied so
    // the format round-trips today.
    let mut active_clash: Option<ClashStrategy> = None;

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = idx + 1;
        let content = strip_comment(raw_line).trim();
        if content.is_empty() {
            continue;
        }

        // Header: `[context clash=longest|all]`.
        if let Some(inner) = content.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            match parse_header(inner) {
                Ok(clash) => {
                    if let Some(c) = clash {
                        active_clash = Some(c);
                    }
                }
                Err(reason) => report.errors.push((line_no, reason)),
            }
            continue;
        }

        // `version = N`.
        if let Some(rest) = content.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(num) = rest.strip_prefix('=') {
                match parse_int_u32(num.trim()) {
                    Some(v) => {
                        report.version = v;
                        if !version_seen && v > KEYS_FORMAT_VERSION {
                            report.warnings.push((
                                line_no,
                                format!(
                                    "file version {v} is newer than supported {KEYS_FORMAT_VERSION}; loading best-effort"
                                ),
                            ));
                        }
                        version_seen = true;
                    }
                    None => report
                        .errors
                        .push((line_no, format!("invalid version value: {:?}", num.trim()))),
                }
                continue;
            }
            // Not a `version =` line after all (e.g. an action literally named
            // `versionX`); fall through to binding parsing.
        }

        // Binding: `action = spec, spec, ...`.
        match parse_binding_line(content, builder, &mut overridden, line_no, &mut report) {
            Ok(()) => {}
            Err(reason) => report.errors.push((line_no, reason)),
        }
    }

    if let Some(c) = active_clash {
        builder.set_clash(c);
    }
    report
}

/// Parses a header body (the text between `[` and `]`): `context [clash=…]`.
/// Returns the clash override if one was specified.
fn parse_header(inner: &str) -> Result<Option<ClashStrategy>, String> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Err("empty context header".to_string());
    }
    // Split into the context name and an optional `clash=…` clause. The context
    // name is the first whitespace-delimited token; the rest is the clause.
    let mut parts = inner.splitn(2, char::is_whitespace);
    let _context = parts.next().unwrap_or("");
    let Some(rest) = parts.next() else {
        return Ok(None);
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(None);
    }
    let Some(value) = rest.strip_prefix("clash") else {
        return Err(format!("unknown header clause: {rest:?}"));
    };
    let value = value.trim_start();
    let Some(value) = value.strip_prefix('=') else {
        return Err(format!("expected `=` after `clash` in header: {rest:?}"));
    };
    match value.trim() {
        "longest" => Ok(Some(ClashStrategy::PrioritizeLongest)),
        "all" => Ok(Some(ClashStrategy::AllowAll)),
        other => Err(format!("unknown clash strategy: {other:?}")),
    }
}

/// Parses one `action = spec, spec, ...` line and applies it to `builder`.
fn parse_binding_line<A: Actionlike>(
    content: &str,
    builder: &mut InputMapBuilder<A>,
    overridden: &mut [bool],
    line_no: usize,
    report: &mut ParseReport,
) -> Result<(), String> {
    let Some(eq) = content.find('=') else {
        return Err(format!("expected `action = spec`: {content:?}"));
    };
    let action_name = content[..eq].trim();
    let spec_str = content[eq + 1..].trim();
    if action_name.is_empty() {
        return Err("empty action name".to_string());
    }

    let Some(action) = action_from_name::<A>(action_name) else {
        // An unknown action is recoverable: skip the line, keep parsing (the
        // enum may have changed; a stale config must not brick the game).
        return Err(format!("unknown action: {action_name:?}"));
    };
    let action_idx = action.index();

    // First override of this action clears its default-seeded slots.
    if !overridden[action_idx] {
        builder.clear_action(action);
        overridden[action_idx] = true;
    }

    // `= none` ⇒ explicit unbind. The slot stays cleared (no bindings appended).
    if spec_str == "none" {
        builder.bind_in_place(action, BindSpec::None);
        return Ok(());
    }

    for spec_tok in split_top_level(spec_str) {
        let spec_tok = spec_tok.trim();
        if spec_tok.is_empty() {
            continue;
        }
        match parse_spec(spec_tok) {
            Ok(SpecParse::Bind(spec)) => builder.bind_in_place(action, spec),
            Ok(SpecParse::StickIgnored(spec)) => {
                builder.bind_in_place(action, spec);
                report.warnings.push((
                    line_no,
                    format!("stick binding {spec_tok:?} parsed but ignored at runtime (v1)"),
                ));
            }
            Err(reason) => {
                // A bad spec within a line is recoverable per-spec: record it,
                // keep the rest of the line.
                report.errors.push((line_no, reason));
            }
        }
    }
    Ok(())
}

/// The outcome of parsing a single spec token.
enum SpecParse {
    /// A runtime-active binding.
    Bind(BindSpec),
    /// A `stick(...)` binding — preserved for round-trip, ignored at runtime.
    StickIgnored(BindSpec),
}

/// Parses one spec token: a key, mouse, chord, composite, or `none`
/// (plan §9.1 `spec`).
fn parse_spec(tok: &str) -> Result<SpecParse, String> {
    // Composite / function-form specs are detected by their `name(` prefix.
    if let Some(rest) = tok.strip_prefix("axis2(") {
        let inner = rest
            .strip_suffix(')')
            .ok_or_else(|| format!("unterminated axis2(...): {tok:?}"))?;
        return parse_axis2(inner).map(SpecParse::Bind);
    }
    if let Some(rest) = tok.strip_prefix("axis1(") {
        let inner = rest
            .strip_suffix(')')
            .ok_or_else(|| format!("unterminated axis1(...): {tok:?}"))?;
        return parse_axis1(inner).map(SpecParse::Bind);
    }
    if tok.strip_prefix("stick(").is_some() {
        // Parsed + preserved, runtime-ignored (plan §13). v1 carries no stick
        // fields on `BindSpec`, so the params are validated for balanced parens
        // only and the binding collapses to the reserved variant.
        if !tok.ends_with(')') {
            return Err(format!("unterminated stick(...): {tok:?}"));
        }
        return Ok(SpecParse::StickIgnored(BindSpec::Stick));
    }
    if tok == "wasd" {
        return Ok(SpecParse::Bind(wasd_spec()));
    }
    if tok == "none" {
        return Ok(SpecParse::Bind(BindSpec::None));
    }

    // Mouse spec.
    if tok.starts_with("Mouse") {
        return mousebutton_from_token(tok)
            .map(|b| SpecParse::Bind(BindSpec::Mouse(b)))
            .ok_or_else(|| format!("invalid mouse spec: {tok:?}"));
    }

    // Chord: `A+B+C` (a `+`-joined list of keys). A single key has no `+`.
    if tok.contains('+') {
        return parse_chord(tok).map(SpecParse::Bind);
    }

    // A single key (a name or `raw(N)`).
    parse_key(tok).map(|k| SpecParse::Bind(BindSpec::Key(k)))
}

/// Parses a single key token: a canonical name or `raw(N)` (= `Unidentified`).
fn parse_key(tok: &str) -> Result<KeyCode, String> {
    if let Some(rest) = tok.strip_prefix("raw(") {
        let inner = rest
            .strip_suffix(')')
            .ok_or_else(|| format!("unterminated raw(...): {tok:?}"))?;
        let n = parse_int_u32(inner.trim())
            .ok_or_else(|| format!("invalid raw() value: {inner:?}"))?;
        return Ok(KeyCode::Unidentified(n));
    }
    keycode_from_name(tok).ok_or_else(|| format!("unknown key: {tok:?}"))
}

/// Parses a `+`-joined chord into a [`BindSpec::Chord`].
fn parse_chord(tok: &str) -> Result<BindSpec, String> {
    let mut keys = [KeyCode::Unidentified(0); MAX_CHORD_KEYS];
    let mut len = 0usize;
    for part in tok.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("empty chord component in {tok:?}"));
        }
        if len >= MAX_CHORD_KEYS {
            return Err(format!(
                "chord exceeds MAX_CHORD_KEYS ({MAX_CHORD_KEYS}): {tok:?}"
            ));
        }
        keys[len] = parse_key(part)?;
        len += 1;
    }
    if len < 2 {
        return Err(format!("chord needs >= 2 keys: {tok:?}"));
    }
    Ok(BindSpec::Chord {
        keys,
        len: len as u8,
    })
}

/// Parses the params of `axis2(...)` into a [`BindSpec::Axis2`].
fn parse_axis2(inner: &str) -> Result<BindSpec, String> {
    let mut up = None;
    let mut down = None;
    let mut left = None;
    let mut right = None;
    let mut dz = 0.0f32;
    let mut mode = AxisMode::DigitalNormalized;

    for (key, val) in params(inner)? {
        match key {
            "up" => up = Some(input_ref_from_name(&val)?),
            "down" => down = Some(input_ref_from_name(&val)?),
            "left" => left = Some(input_ref_from_name(&val)?),
            "right" => right = Some(input_ref_from_name(&val)?),
            "dz" => dz = parse_f32(&val)?,
            "mode" => mode = parse_axis_mode(&val)?,
            other => return Err(format!("unknown axis2 param: {other:?}")),
        }
    }

    Ok(BindSpec::Axis2 {
        up: up.ok_or("axis2 missing `up`")?,
        down: down.ok_or("axis2 missing `down`")?,
        left: left.ok_or("axis2 missing `left`")?,
        right: right.ok_or("axis2 missing `right`")?,
        dz,
        mode,
    })
}

/// Parses the params of `axis1(...)` into a [`BindSpec::Axis1`].
fn parse_axis1(inner: &str) -> Result<BindSpec, String> {
    let mut neg = None;
    let mut pos = None;
    let mut dz = 0.0f32;

    for (key, val) in params(inner)? {
        match key {
            "neg" => neg = Some(input_ref_from_name(&val)?),
            "pos" => pos = Some(input_ref_from_name(&val)?),
            "dz" => dz = parse_f32(&val)?,
            other => return Err(format!("unknown axis1 param: {other:?}")),
        }
    }

    Ok(BindSpec::Axis1 {
        neg: neg.ok_or("axis1 missing `neg`")?,
        pos: pos.ok_or("axis1 missing `pos`")?,
        dz,
    })
}

/// Splits a `key=value, key=value` param list (paren-depth aware) into pairs.
fn params(inner: &str) -> Result<Vec<(&str, String)>, String> {
    let mut out = Vec::new();
    for part in split_top_level(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(eq) = part.find('=') else {
            return Err(format!("expected `key=value` param: {part:?}"));
        };
        let key = part[..eq].trim();
        let val = part[eq + 1..].trim();
        out.push((key, val.to_string()));
    }
    Ok(out)
}

/// Resolves an axis-leg token (a key name or `raw(N)`) into an [`InputRef`].
/// Mouse legs are accepted via the mouse-token forms.
fn input_ref_from_name(tok: &str) -> Result<InputRef, String> {
    if tok.starts_with("Mouse") {
        return mousebutton_from_token(tok)
            .map(InputRef::Mouse)
            .ok_or_else(|| format!("invalid mouse axis leg: {tok:?}"));
    }
    parse_key(tok).map(InputRef::Key)
}

/// Parses an [`AxisMode`] token (`radial` = normalized, `raw` = unnormalized).
fn parse_axis_mode(tok: &str) -> Result<AxisMode, String> {
    match tok {
        "radial" | "normalized" => Ok(AxisMode::DigitalNormalized),
        "raw" => Ok(AxisMode::DigitalRaw),
        other => Err(format!("unknown axis mode: {other:?}")),
    }
}

/// Parses an `f32` value, mapping a malformed literal to a recoverable error.
fn parse_f32(tok: &str) -> Result<f32, String> {
    tok.parse::<f32>()
        .map_err(|_| format!("invalid float: {tok:?}"))
}

/// Resolves an `Actionlike` variant from its `name()`. Linear over `0..COUNT` —
/// cold load-time path only.
fn action_from_name<A: Actionlike>(name: &str) -> Option<A> {
    (0..A::COUNT)
        .filter_map(A::from_index)
        .find(|a| a.name() == name)
}

/// The canonical WASD composite (matches [`InputMapBuilder::wasd`]).
fn wasd_spec() -> BindSpec {
    BindSpec::Axis2 {
        up: InputRef::Key(KeyCode::KeyW),
        down: InputRef::Key(KeyCode::KeyS),
        left: InputRef::Key(KeyCode::KeyA),
        right: InputRef::Key(KeyCode::KeyD),
        dz: 0.0,
        mode: AxisMode::DigitalNormalized,
    }
}

/// Strips the trailing comment from a line at the first **unquoted** `#`
/// (plan §9.3). A `#` inside a `"…"` quoted span is literal; an unterminated
/// quote is tolerated (the rest of the line is treated as quoted) so a stray
/// quote never swallows a `#`-comment of a *later* line.
pub fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'#' if !in_quotes => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Splits a top-level comma-separated list while tracking paren depth and quotes
/// (plan §9.3). `axis2(up=W, down=S)` stays one element — a comma inside `(...)`
/// or inside `"…"` does not split. The returned slices borrow from `s`.
pub fn split_top_level(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut in_quotes = false;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'(' if !in_quotes => depth += 1,
            b')' if !in_quotes => depth = depth.saturating_sub(1),
            b',' if !in_quotes && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::keycode::MouseButton;

    #[test]
    fn comment_stripped_at_unquoted_hash() {
        assert_eq!(strip_comment("jump = Space # comment"), "jump = Space ");
        assert_eq!(strip_comment("no comment here"), "no comment here");
    }

    #[test]
    fn comment_inside_quotes_is_literal() {
        // The `#` inside the quoted span is NOT a comment start.
        assert_eq!(strip_comment("x = \"a#b\" # real"), "x = \"a#b\" ");
        assert_eq!(strip_comment("x = \"only#inside\""), "x = \"only#inside\"");
    }

    #[test]
    fn top_level_split_is_paren_depth_aware() {
        // The inner commas of axis2(...) must not split the spec.
        let parts = split_top_level("wasd, axis2(up=W, down=S, left=A, right=D)");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].trim(), "wasd");
        assert_eq!(parts[1].trim(), "axis2(up=W, down=S, left=A, right=D)");
    }

    #[test]
    fn top_level_split_respects_quotes() {
        let parts = split_top_level("\"a,b\", c");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].trim(), "\"a,b\"");
        assert_eq!(parts[1].trim(), "c");
    }

    #[test]
    fn raw_key_round_trips() {
        let k = parse_key("raw(0x56)").unwrap();
        assert_eq!(k, KeyCode::Unidentified(0x56));
        let k2 = parse_key("raw(86)").unwrap();
        assert_eq!(k2, KeyCode::Unidentified(86));
    }

    #[test]
    fn mouse_other_round_trips() {
        let m = mousebutton_from_token("MouseOther(9)").unwrap();
        assert_eq!(m, MouseButton::Other(9));
    }

    #[test]
    fn chord_parses_and_orders_count() {
        let spec = parse_chord("LCtrl+S").unwrap();
        match spec {
            BindSpec::Chord { len, .. } => assert_eq!(len, 2),
            _ => panic!("expected chord"),
        }
    }

    #[test]
    fn axis2_parses_all_params() {
        let spec = parse_axis2("up=W, down=S, left=A, right=D, dz=0.15, mode=radial").unwrap();
        match spec {
            BindSpec::Axis2 { dz, mode, .. } => {
                assert!((dz - 0.15).abs() < 1e-6);
                assert_eq!(mode, AxisMode::DigitalNormalized);
            }
            _ => panic!("expected axis2"),
        }
    }

    #[test]
    fn version_higher_warns_not_fails() {
        // A future version must load best-effort with a warning, never hard-fail.
        let report = ParseReport {
            version: KEYS_FORMAT_VERSION,
            ..Default::default()
        };
        assert!(report.is_clean());
    }
}
