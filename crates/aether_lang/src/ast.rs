//! The Aether AST — one node per construct, kept deliberately shallow.
//!
//! Bodies, types and paths are **verbatim `syn` fragments with the user's spans** (§2): the AST
//! carries structure, never re-lexed text, so downstream rustc errors land on the user's tokens.

use syn::{Ident, Path, Type};

/// One parsed `aether!` block: a flat list of constructs sharing one parse context.
pub struct AetherBlock {
    /// The constructs, in source order (emission preserves it — deterministic output is what
    /// makes the unit-test snapshots exact).
    pub constructs: Vec<Construct>,
}

/// Every construct rung A0 knows. Later rungs add variants — one new variant, one new parser
/// row, one new expander module each (§6.1's extensibility contract).
pub enum Construct {
    /// `component NAME { fields / requires / hooks / no_bundle }`
    Component(ComponentDef),
    /// `tag NAME;` / `tag NAME(bitset);`
    Tag(TagDef),
}

/// The four Phase-14a hook keys the `component` construct forwards (§3.1). Mutually exclusive
/// with the runtime builder — the DERIVE enforces that; Aether just forwards the pairs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// `on_add = path`
    Add,
    /// `on_insert = path`
    Insert,
    /// `on_replace = path`
    Replace,
    /// `on_remove = path`
    Remove,
}

impl HookKind {
    /// The derive-attribute key this hook emits (`#[component(<key> = path)]`).
    pub fn key(self) -> &'static str {
        match self {
            HookKind::Add => "on_add",
            HookKind::Insert => "on_insert",
            HookKind::Replace => "on_replace",
            HookKind::Remove => "on_remove",
        }
    }

    /// Parse a hook key from its surface ident, `None` for anything else.
    pub fn from_str(s: &str) -> Option<HookKind> {
        match s {
            "on_add" => Some(HookKind::Add),
            "on_insert" => Some(HookKind::Insert),
            "on_replace" => Some(HookKind::Replace),
            "on_remove" => Some(HookKind::Remove),
            _ => None,
        }
    }
}

/// `component NAME { … }` (§3.1).
pub struct ComponentDef {
    /// The type name (uppercase by the §2 case convention — diagnosed at parse).
    pub name: Ident,
    /// `field: Type,` pairs, verbatim types.
    pub fields: Vec<(Ident, Type)>,
    /// `requires A, B::C,` — accumulated across multiple `requires` items.
    pub requires: Vec<Path>,
    /// `on_add = path,` etc. Duplicate KEYS are diagnosed at parse (the derive would too, but
    /// the parser owns the better span — the §3.1 pre-check rule).
    pub hooks: Vec<(HookKind, Path)>,
    /// `no_bundle,` — suppresses the Phase-22 single-component `Bundle` emission.
    pub no_bundle: bool,
}

/// `tag NAME;` / `tag NAME(bitset);` (§3.1) — a zero-data marker.
pub struct TagDef {
    /// The type name.
    pub name: Ident,
    /// `(bitset)` — the EnableTag backend (`storage = "bitset"`): O(1) toggle, no migration.
    pub bitset: bool,
}
