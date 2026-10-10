//! The one-pass indentation parser (P3 §A, Decision 1 / 2 / 6).
//!
//! `.ui` is an off-side-rule indentation tree. A single stack machine classifies
//! every line with ZERO lookahead (Decision 1): each open node records its own
//! `head_indent`, so a component line is classified purely by the integer
//! relation of its indent to the stack-top node's `head_indent + STEP` —
//! `#`-lead is always a named head; an IDENT-lead at `rel == STEP` is an
//! attached component; an IDENT-lead at `rel == 0` is an anonymous child head.
//!
//! Recovery (Decision 6): the parser NEVER fails at the file level. A malformed
//! construct is recorded in the [`UiParseReport`] with `(line, col, reason)` and
//! skipped; the rest parses.

// `.ui` source parsing runs at asset load / hot-reload only (the watch system reaches
// `parse_ui` solely after a confirmed mtime+size change), never on a per-frame path. The
// duplicate-`#name` dedup set is a per-parse scratch structure over owned `String` keys,
// discarded when the transient `ParsedTree` is consumed.
#[allow(clippy::disallowed_types)]
use std::collections::HashSet;

use crate::components::UiName;
use crate::text::ast::{CompKind, ParsedComponent, ParsedNode, ParsedTree, UiNameStr};
use crate::text::report::{UiParseReport, UI_FORMAT_VERSION};
use crate::text::split::{
    extract_component_span, indent_is_consistent, leading_ws_width, strip_comment_slashslash,
    CompSpan, STEP,
};

/// A `usize` sentinel marking the bottom-of-stack root level (no enclosing
/// node). The first real node opens against this frame at indent 0.
const ROOT_SENTINEL: usize = usize::MAX;

/// One open-ancestor frame on the indent stack.
struct StackFrame {
    /// The head indentation of the node this frame represents.
    head_indent: u32,
    /// Index into the arena, or [`ROOT_SENTINEL`] for the bottom frame.
    node_index: usize,
    /// The next child-declaration ordinal to assign under this node.
    next_ordinal: u32,
}

/// Parse `.ui` source into a transient tree + a recoverable error report.
/// Never fails at the file level (Decision 6).
// Load / hot-reload path only; `seen_names` is per-parse scratch (see the `use` above).
#[allow(clippy::disallowed_types)]
pub fn parse_ui(src: &str) -> ParsedTree {
    let mut nodes: Vec<ParsedNode> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    let mut report = UiParseReport::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut version_seen = false;

    // The bottom sentinel: the first real node (indent 0) opens at rel == STEP
    // relative to head_indent == 0? No — a top-level node sits at indent 0 and
    // must be a child of the sentinel. The sentinel uses head_indent that makes
    // `rel == STEP` for an indent-0 line: head_indent = 0 means rel == 0, which
    // we treat as "child of the sentinel" specially (a root). To keep the
    // uniform `rel ∈ {0, STEP}` test, the sentinel records head_indent such that
    // an indent-0 head is its child; we handle the sentinel explicitly below.
    let mut stack: Vec<StackFrame> = vec![StackFrame {
        head_indent: 0,
        node_index: ROOT_SENTINEL,
        next_ordinal: 0,
    }];

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = idx + 1;
        let content = strip_comment_slashslash(raw_line);
        let indent = leading_ws_width(content);
        let body = content.trim();
        if body.is_empty() {
            continue; // blank / comment-only: no INDENT / DEDENT
        }

        // version=N — mirror the `.keys` flow exactly (strip_prefix + require `=`).
        if !version_seen
            && let Some(rest) = body.strip_prefix("version")
        {
            let rest = rest.trim_start();
            if let Some(num) = rest.strip_prefix('=') {
                match num.trim().parse::<u32>() {
                    Ok(v) => {
                        report.version = v;
                        if v > UI_FORMAT_VERSION {
                            report.warn(
                                line_no,
                                0,
                                format!(
                                    "file version {v} is newer than supported {UI_FORMAT_VERSION}; loading best-effort"
                                ),
                            );
                        }
                        version_seen = true;
                    }
                    Err(_) => report.error(line_no, 0, "invalid version value"),
                }
                continue;
            }
            // A token literally starting with "version" but not "version=" (an
            // unlikely component name): fall through to node parsing.
        }

        // Indentation must be spaces-only and a multiple of STEP (Decision 6).
        if !indent_is_consistent(content) {
            report.error(line_no, 0, "inconsistent indentation (use spaces, a multiple of 4)");
            continue;
        }

        // DEDENT: pop frames deeper than this line's indent. The bottom sentinel
        // is never popped.
        while stack.len() > 1 && indent < stack_top(&stack).head_indent {
            stack.pop();
        }

        // Post-DEDENT alignment check (Decision 6). `rel` is the line's indent
        // relative to the stack-top node's head. For the bottom sentinel
        // (head_indent 0, node ROOT_SENTINEL) an indent-0 line is a root.
        let top = stack_top(&stack);
        let is_sentinel = top.node_index == ROOT_SENTINEL;
        let rel = indent as i64 - top.head_indent as i64;

        if is_sentinel {
            // Only an indent-0 line is valid against the sentinel (a root head).
            if rel != 0 {
                report.error(line_no, indent as u16, "unexpected indentation at the document root");
                continue;
            }
        } else if rel != 0 && rel != STEP as i64 {
            // Either a sibling of the stack-top node (rel == 0), or one nesting
            // step deeper (rel == STEP: attached component OR child head).
            // Anything else is a dedent to a never-opened column → record + SKIP
            // WITHOUT mutating the stack so siblings still parse correctly.
            report.error(
                line_no,
                indent as u16,
                "indent does not align to a sibling or a single nesting step",
            );
            continue;
        }

        let line_base_col = indent as u16;

        if body.starts_with('#') {
            // ── A NAMED node head. ───────────────────────────────────────────
            // rel == 0 against a NON-sentinel top means a sibling: pop the
            // sibling frame so the new node attaches to the shared parent. rel ==
            // STEP (or rel == 0 against the sentinel) means a child of the top.
            if !is_sentinel && rel == 0 {
                stack.pop();
            }
            let parent_index = stack_top(&stack).node_index;
            let ordinal = next_ordinal(&mut stack);

            let (name, rest) = split_name(body, line_no, &mut report, &mut seen_names);
            let node_index = nodes.len();
            let mut node = ParsedNode::new(name, indent, ordinal, line_no);

            // Optional inline component after the name.
            let rest = rest.trim();
            if !rest.is_empty() {
                push_component(&mut node, rest, line_base_col, line_no, &mut report);
            }

            nodes.push(node);
            link(&mut nodes, &mut roots, parent_index, node_index);
            stack.push(StackFrame { head_indent: indent, node_index, next_ordinal: 0 });
        } else {
            // ── An IDENT-lead line: attached component OR anonymous child head. ─
            if !is_sentinel && rel == STEP as i64 {
                // Attached component of the stack-top node (Decision 1). Does NOT
                // push a stack frame (a component is not a nesting node).
                let top_index = stack_top(&stack).node_index;
                debug_assert!(top_index != ROOT_SENTINEL, "attached component requires a real parent");
                // SAFETY of index: `top_index` is a real node pushed earlier.
                let node = &mut nodes[top_index];
                push_component(node, body, line_base_col, line_no, &mut report);
            } else {
                // rel == 0 (sibling level), OR an indent-0 line at the sentinel:
                // an ANONYMOUS child node head whose head IS this component.
                if !is_sentinel && rel == 0 {
                    stack.pop();
                }
                let parent_index = stack_top(&stack).node_index;
                let ordinal = next_ordinal(&mut stack);
                let node_index = nodes.len();
                let mut node = ParsedNode::new(None, indent, ordinal, line_no);
                push_component(&mut node, body, line_base_col, line_no, &mut report);
                // Decision 11 nudge: an anonymous node carrying a non-UiLayout
                // component is state-fragile across reloads.
                if node.components.iter().any(|c| c.name != "UiLayout") {
                    report.warn(
                        line_no,
                        line_base_col,
                        "anonymous node carries non-UiLayout state; prefer a #name for stable reload",
                    );
                }
                nodes.push(node);
                link(&mut nodes, &mut roots, parent_index, node_index);
                stack.push(StackFrame { head_indent: indent, node_index, next_ordinal: 0 });
            }
        }
    }

    // An anonymous node that later gains children is also state-fragile.
    for node in &nodes {
        if node.name.is_none() && !node.children.is_empty() {
            report.warn(
                node.line_no,
                node.head_indent as u16,
                "anonymous node has children; prefer a #name for stable reload",
            );
        }
    }

    ParsedTree { nodes, roots, report }
}

/// Returns a reference to the current stack top (the stack always holds the
/// bottom sentinel, so this never underflows).
#[inline]
fn stack_top(stack: &[StackFrame]) -> &StackFrame {
    debug_assert!(!stack.is_empty(), "invariant: indent stack retains the bottom sentinel");
    // The bottom sentinel guarantees a non-empty stack at all times.
    &stack[stack.len() - 1]
}

/// Assigns and bumps the next child-declaration ordinal on the current top.
#[inline]
fn next_ordinal(stack: &mut [StackFrame]) -> u32 {
    let last = stack.len() - 1;
    let frame = &mut stack[last];
    let o = frame.next_ordinal;
    frame.next_ordinal += 1;
    o
}

/// Links `child_index` under `parent_index` (or as a root for the sentinel).
#[inline]
fn link(nodes: &mut [ParsedNode], roots: &mut Vec<usize>, parent_index: usize, child_index: usize) {
    if parent_index == ROOT_SENTINEL {
        roots.push(child_index);
    } else {
        nodes[parent_index].children.push(child_index);
    }
}

/// Splits a `#name [rest]` head into the (optional, validated) name and the
/// trailing component text. Demotes a duplicate or over-CAP name to anonymous
/// (Decision 6), recording the error.
// Borrows `parse_ui`'s per-parse dedup scratch; load / hot-reload path only.
#[allow(clippy::disallowed_types)]
fn split_name<'a>(
    body: &'a str,
    line_no: usize,
    report: &mut UiParseReport,
    seen_names: &mut HashSet<String>,
) -> (Option<UiNameStr>, &'a str) {
    debug_assert!(body.starts_with('#'));
    let after_hash = &body[1..];
    // The name is the leading run of identifier-ish bytes; the rest is the
    // (optional) inline component.
    let name_end = after_hash
        .find(|c: char| c.is_whitespace())
        .unwrap_or(after_hash.len());
    let name = &after_hash[..name_end];
    let rest = &after_hash[name_end..];

    if name.is_empty() {
        report.error(line_no, 1, "empty #name");
        return (None, rest);
    }
    if name.len() > UiName::CAP {
        report.error(
            line_no,
            1,
            format!("ui name `{name}` exceeds {} bytes; demoted to anonymous", UiName::CAP),
        );
        return (None, rest);
    }
    if !seen_names.insert(name.to_string()) {
        report.error(
            line_no,
            1,
            format!("duplicate ui name `{name}`; demoted to anonymous"),
        );
        return (None, rest);
    }
    (Some(UiNameStr::new(name.to_string())), rest)
}

/// Extracts and pushes ONE component literal from `text` (already trimmed of
/// leading whitespace) onto `node`. Records a recoverable error if the
/// component span is malformed (unterminated brace/paren, non-ident head).
fn push_component(
    node: &mut ParsedNode,
    text: &str,
    line_base_col: u16,
    line_no: usize,
    report: &mut UiParseReport,
) {
    // The base col of `text` within the line: leading-ws offset + any name
    // prefix already consumed. The caller passes `line_base_col` = the line's
    // indent width; `text` begins at `indent + (body_offset)`. We recompute the
    // body offset from the trim difference is unnecessary — `extract_component_span`
    // reports columns relative to `text`'s own start, offset by the base.
    match extract_component_span(text, line_base_col) {
        Some(CompSpan::Struct { name, body, body_col }) => {
            node.components.push(ParsedComponent {
                name: name.to_string(),
                body: body.to_string(),
                kind: CompKind::Struct,
                line_no,
                body_col,
            });
        }
        Some(CompSpan::Tuple { name, body, body_col }) => {
            node.components.push(ParsedComponent {
                name: name.to_string(),
                body: body.to_string(),
                kind: CompKind::Tuple,
                line_no,
                body_col,
            });
        }
        Some(CompSpan::Bare { name }) => {
            // Reject trailing junk after a bare IDENT (a bare component line is
            // just the IDENT, e.g. `UiRoot`).
            if text.trim() != name {
                report.error(line_no, line_base_col, "trailing text after a marker component");
                return;
            }
            node.components.push(ParsedComponent {
                name: name.to_string(),
                body: String::new(),
                kind: CompKind::Bare,
                line_no,
                body_col: line_base_col,
            });
        }
        None => {
            report.error(line_no, line_base_col, "malformed component (expected `Name { .. }`)");
        }
    }
    let _ = report; // keep the borrow shape uniform across arms
}
