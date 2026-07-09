//! The transient parsed-node tree for a `.ui` document (P3 §Data structures).
//!
//! Cold path: this is built once per load / reload and freely allocates. The
//! tree is arena-flat — children are stored by index into one contiguous `Vec`,
//! so the recursive walk (lowering / serialize) is index-based and there is no
//! `Box`, no per-node heap node. `ParsedComponent.body` is kept as an owned
//! `String` (parsed to the typed component only at lowering time) so the same
//! AST round-trips through the serializer.

use crate::components::UiName;
use crate::text::report::UiParseReport;

/// Whether a parsed component literal used the brace struct form or the paren
/// tuple form (Decision 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompKind {
    /// `IDENT { field: value, ... }` — the struct form.
    Struct,
    /// `IDENT ( value )` — the tuple-newtype form (e.g. `StackIndex(10)`).
    Tuple,
    /// `IDENT` — a ZST marker component (e.g. `UiRoot`).
    Bare,
}

/// A parsed component literal: its text name + the raw field/arg span. The body
/// is parsed to the typed component at lowering time.
#[derive(Clone, Debug)]
pub struct ParsedComponent {
    /// The component's text name — the closed-vocabulary dispatch key
    /// (Decision 3). By invariant it equals the Rust type name.
    pub name: String,
    /// The raw inner span: `"width: Px(80), height: Px(24)"` (Struct) or `"10"`
    /// (Tuple) or `""` (Bare).
    pub body: String,
    /// Distinguishes the struct / tuple / bare forms.
    pub kind: CompKind,
    /// 1-based source line this component was declared on.
    pub line_no: usize,
    /// 0-based byte column of the body start within its line (for field-error
    /// locality).
    pub body_col: u16,
}

/// A validated name that fits [`UiName::CAP`], captured at parse time (the
/// runtime bound-check the text path is responsible for, since the `ui!` macro
/// enforces it at compile time instead).
#[derive(Clone, Debug)]
pub struct UiNameStr {
    /// The name text; `text.len() <= UiName::CAP` is a parser invariant.
    pub text: String,
}

impl UiNameStr {
    /// Wraps `text` after asserting (debug-only) the parser already bounded it.
    #[inline]
    pub(crate) fn new(text: String) -> Self {
        debug_assert!(
            text.len() <= UiName::CAP,
            "invariant: UiNameStr exceeds UiName::CAP (parser must bound before constructing)"
        );
        Self { text }
    }
}

/// A node in the transient parse tree (arena-flat — `children` are indices into
/// [`ParsedTree::nodes`]).
#[derive(Clone, Debug)]
pub struct ParsedNode {
    /// The `#name`, or `None` for an anonymous node (or a name-demoted
    /// duplicate, Decision 6).
    pub name: Option<UiNameStr>,
    /// The node's attached components, in declaration order.
    pub components: Vec<ParsedComponent>,
    /// Child node indices into the arena, in declaration order.
    pub children: Vec<usize>,
    /// Declaration ordinal among this node's siblings (0-based, per-parent) —
    /// the stable anonymous-reconcile key (Decision 11), stamped at lowering as
    /// `UiSourceOrder`.
    pub sibling_ordinal: u32,
    /// The node head's leading indentation width — used for lookahead-free
    /// classification (Decision 1).
    pub head_indent: u32,
    /// 1-based source line of the node's head.
    pub line_no: usize,
}

impl ParsedNode {
    /// Constructs an empty node (no components, no children) at `head_indent`.
    #[inline]
    pub(crate) fn new(
        name: Option<UiNameStr>,
        head_indent: u32,
        sibling_ordinal: u32,
        line_no: usize,
    ) -> Self {
        Self {
            name,
            components: Vec::new(),
            children: Vec::new(),
            sibling_ordinal,
            head_indent,
            line_no,
        }
    }
}

/// The parse result: a flat node arena + the root indices + the recoverable
/// report. `nodes[roots[i]]` are the document's roots.
#[derive(Clone, Debug)]
pub struct ParsedTree {
    /// The arena of all parsed nodes.
    pub nodes: Vec<ParsedNode>,
    /// Indices into `nodes` of the document's roots, in declaration order.
    pub roots: Vec<usize>,
    /// Recoverable errors + warnings + the parsed version.
    pub report: UiParseReport,
}

impl ParsedTree {
    /// `true` when the document declared no nodes (e.g. an empty file or one
    /// that was only `version=1`).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}
