//! Lexical scanning primitives for the `.ui` parser (P3 Decision 2 / 5 / 15).
//!
//! All functions here are pure, allocation-light, and operate on byte slices of
//! a single source line. They are the cold load-time scanning layer shared by
//! the indentation parser (`parser.rs`) and the per-component field extractor.
//!
//! # What is copied vs. new
//!
//! * [`split_top_level`] was copied from the `.keys` parser
//!   (`boyko_input::persist::grammar::split_top_level`) and has since DIVERGED —
//!   see its own doc. Copying (not depending on `boyko_input`) avoids a
//!   `boyko_ui → boyko_input` crate edge for a ~20-line pure function. It tracks
//!   paren + BRACKET depth and quote state; it is NOT brace-aware, so P3 uses it
//!   strictly on the already-extracted INNER field list (Decision 5), never to
//!   isolate a component span.
//! * [`strip_comment_slashslash`] is NEW: the `.keys` strip-comment is a
//!   single-byte `#` rule, but `.ui` reserves `#` for the name sigil, so the
//!   comment lead is the two-byte `//` (Decision 2). It returns the PRE-TRIM
//!   slice (the `.keys` strip-then-trim flow is intentionally NOT reused).
//! * [`leading_ws_width`], [`indent_is_consistent`], [`extract_component_span`]
//!   are NEW (Decision 1 / 5 / 15).

/// The canonical indentation step: 4 spaces per nesting level (P3 §1).
pub(crate) const STEP: u32 = 4;

/// Splits a top-level comma-separated list while tracking paren and BRACKET
/// depth plus quotes. A comma inside `(...)`, inside `[...]`, or inside `"…"`
/// does not split. The returned slices borrow from `s`.
///
/// Used ONLY on the inner field list of a component body (Decision 5), never to
/// isolate a component span — that is [`extract_component_span`]'s brace-matching
/// job.
///
/// # The bracket rule is a FIX, and the doc it replaces was false twice
///
/// This function was copied from
/// `boyko_input::persist::grammar::split_top_level`, whose grammar has no
/// bracketed values, and its doc claimed the P3 field list was *"provably free of
/// `{`/`[`/quoted-comma values … locked by a rejection test"*. Neither half held:
/// GUI P6a added `UiImage`'s `uv_min`/`uv_max`, which are `[u, v]`, and no such
/// rejection test exists anywhere in the tree.
///
/// The consequence was silent. MEASURED at the UI-ADVANCED S6 build: a `.ui`
/// source spelling `UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint: … }`
/// split into `uv_min: [0` / `0]` / `uv_max: [1` / `1]`, so `parse_f32_pair`
/// rejected both UV fields, they kept their `Default`s, and four recoverable
/// errors went into the LOWERING report — the report `p3_common::spawn_dot_ui`
/// clones and drops. `p6a_equivalence::image_widget_three_ways_equivalent` was
/// green over it only because the authored UVs happened to EQUAL the defaults
/// (`[0,0]`/`[1,1]`). Bracketed values in a `.ui` file had never parsed.
///
/// # One depth counter, not two
///
/// `(` and `[` both open and `)` and `]` both close the same counter. A crossed
/// pair (`[a)`) is not diagnosed here — it is a malformed value that the
/// type-directed leaf parser rejects one step later with a per-field error, which
/// is the layer that owns value shape. Two independent counters would trade one
/// pathological mis-split for another, at the same cost.
pub(crate) fn split_top_level(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut in_quotes = false;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'(' | b'[' if !in_quotes => depth += 1,
            b')' | b']' if !in_quotes => depth = depth.saturating_sub(1),
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

/// Strips a trailing comment from a line at the first **unquoted** `//`
/// (two consecutive `/`), returning the PRE-TRIM slice (Decision 2).
///
/// A `#` is NEVER a comment (it is the `.ui` name sigil); a lone `/` (e.g. a
/// future path-like `a/b`) is literal. A `//` inside a `"…"` span is literal —
/// P3 has no quoted string values yet, but the quote machinery is retained
/// verbatim so P5b (quoted SDF text) inherits it without a rewrite. An
/// unterminated quote is tolerated (the rest of the line is treated as quoted),
/// so a stray quote never swallows the `//`-comment of a LATER line.
///
/// Unlike the `.keys` strip-then-trim sequence, this returns the un-trimmed
/// slice so the caller can measure leading indentation on the same slice
/// ([`leading_ws_width`]) before trimming the body.
pub(crate) fn strip_comment_slashslash(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            // A comment begins at the first unquoted `//`. Guard `i + 1` so a
            // trailing lone `/` at end-of-line is never read out of bounds.
            b'/' if !in_quotes && i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return &line[..i];
            }
            _ => {}
        }
        i += 1;
    }
    line
}

/// Counts the leading ASCII-space width of `line` (Decision 1).
///
/// Measured on the comment-stripped, UN-trimmed slice. A `\t` is NOT counted as
/// a space — a tab in the indentation is flagged inconsistent by
/// [`indent_is_consistent`], so this only ever runs on a space-only prefix in
/// the canonical case.
pub(crate) fn leading_ws_width(line: &str) -> u32 {
    let mut n = 0u32;
    for &b in line.as_bytes() {
        if b == b' ' {
            n += 1;
        } else {
            break;
        }
    }
    n
}

/// Validates that the indentation of `line` is spaces-only and a whole multiple
/// of [`STEP`] (Decision 6 indentation-consistency check).
///
/// Returns `false` (inconsistent) if the leading whitespace contains a tab, or
/// if the space count is not a multiple of `STEP`. The body content past the
/// indent is not examined.
pub(crate) fn indent_is_consistent(line: &str) -> bool {
    let mut spaces = 0u32;
    for &b in line.as_bytes() {
        match b {
            b' ' => spaces += 1,
            b'\t' => return false, // mixed tab/space in the indent
            _ => break,
        }
    }
    spaces.is_multiple_of(STEP)
}

/// A component literal split into its name and body span, distinguished by the
/// delimiter form (Decision 5 / 15).
pub(crate) enum CompSpan<'a> {
    /// `IDENT { field_list }` — the brace struct form. `body` is the inner
    /// field list (between the outer `{` and `}`), with leading/trailing
    /// whitespace untrimmed; `body_col` is its byte offset within the line.
    Struct { name: &'a str, body: &'a str, body_col: u16 },
    /// `IDENT ( value )` — the tuple newtype form (e.g. `StackIndex(10)`).
    /// `body` is the single arg between `(` and `)`; `body_col` is its offset.
    Tuple { name: &'a str, body: &'a str, body_col: u16 },
    /// `IDENT` with no delimiter — a ZST marker component (e.g. `UiRoot`).
    Bare { name: &'a str },
}

/// Extracts ONE component literal from `s`, where `s` is the text AFTER any
/// `#name` prefix (the head's inline component, or a stand-alone attached
/// component line). `s` is expected to be trimmed of leading whitespace by the
/// caller (the parser trims the body before classification).
///
/// This is the brace/paren-MATCHING component-span scan (Decision 5): it finds
/// the component identifier, then a quote-aware depth scan locates the matching
/// closing delimiter so the inner span is isolated WITHOUT a comma split (the
/// hole the `split_top_level`-on-the-whole-line approach would leave open).
///
/// Returns `None` if `s` is empty or does not begin with an identifier byte.
/// An unterminated `{`/`(` yields `None` (a recoverable parse error at the
/// call site).
pub(crate) fn extract_component_span(s: &str, line_base_col: u16) -> Option<CompSpan<'_>> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    // The identifier: `[A-Za-z_][A-Za-z0-9_]*`. The first byte must be an ident
    // start; a leading digit / brace / paren is not a component head.
    if i >= bytes.len() || !is_ident_start(bytes[i]) {
        return None;
    }
    while i < bytes.len() && is_ident_continue(bytes[i]) {
        i += 1;
    }
    let name = &s[..i];

    // Skip optional spaces between the name and the delimiter (`UiLayout {` and
    // `UiLayout{` both accepted; `StackIndex (10)` and `StackIndex(10)` both
    // accepted).
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }

    match bytes.get(i) {
        Some(b'{') => {
            let open = i;
            let close = match_delim(bytes, open, b'{', b'}')?;
            let body = &s[open + 1..close];
            let body_col = line_base_col.saturating_add((open + 1) as u16);
            Some(CompSpan::Struct { name, body, body_col })
        }
        Some(b'(') => {
            let open = i;
            let close = match_delim(bytes, open, b'(', b')')?;
            let body = &s[open + 1..close];
            let body_col = line_base_col.saturating_add((open + 1) as u16);
            Some(CompSpan::Tuple { name, body, body_col })
        }
        // No delimiter (or trailing junk): a bare marker IDENT. Trailing
        // non-whitespace after the IDENT is rejected at the call site (a bare
        // component line is just the IDENT).
        _ => Some(CompSpan::Bare { name }),
    }
}

/// Finds the index of the delimiter matching the opener at `open` (which must be
/// `open_byte`), tracking nesting depth and quote state. Returns the index of
/// the matching `close_byte`, or `None` if unterminated.
fn match_delim(bytes: &[u8], open: usize, open_byte: u8, close_byte: u8) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_quotes = false;
    let mut i = open;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            if b == open_byte {
                depth += 1;
            } else if b == close_byte {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Whether `b` may begin an identifier.
#[inline]
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Whether `b` may continue an identifier.
#[inline]
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
