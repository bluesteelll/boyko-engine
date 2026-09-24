//! Leg (1) of UG-15, the source census (03 §6), and the walker the maps and leg (5) reuse.
//!
//! **One visitor, three inputs.** [`count_file`] walks a parsed file: every item, including
//! items nested inside fn bodies and const blocks, impl items, trait items and foreign items.
//! It is run over the six census crates' `src/**` (the leg), over the expanded output of every
//! `boyko_macros` fixture (M-b, from that crate's own tests), and over a build script's generated
//! sources (`ug15 leg7b`). A second copy of the visitor would be a second definition of
//! "counted" that can drift from the first.
//!
//! **Counted** (03 §6 leg (1)):
//! - (a) an attribute whose path is one of [`COUNTED_ATTRS`], written plainly, inside Rust 2024's
//!   `unsafe(...)`, or inside a `cfg_attr(pred, ...)` list, at any depth (PC-3 c);
//! - (b) a fn DEFINITION with an explicit ABI other than `"Rust"`, or a bare `extern fn`;
//! - (c) `global_asm!`, `naked_asm!` (matched by the macro path's last segment, so
//!   `core::arch::global_asm!` counts) and `#[naked]`.
//!
//! **Not counted:** `extern "…" { … }` import blocks, fn-pointer types, `extern crate`, `asm!`.
//!
//! **Macro bodies are opaque to `syn`** (`syn::Macro::tokens`). M-a ([`scan_tokens`]) therefore
//! matches token sequences inside every macro body the walk meets — `quote!` templates, every
//! invocation, and every `macro_rules!` body (PC-3 a: 72 of them in the census crates, 33
//! exported). A `#` followed by an identifier is quote interpolation and never matches, which is
//! why control (vi) (an attribute built with `format_ident!`) needs M-b.
//!
//! **A build script's string literals** ([`scan_build_rs`]) are the templates of the code it
//! generates (PC-3 b: `boyko_diag/build.rs` writes `profile_axis.rs`, which `profile.rs`
//! `include!`s). A literal that tokenizes is scanned with M-a; one that does not is matched as
//! text against attribute-shaped patterns, never skipped (critique O3).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::{self, Visit};

use crate::json::Json;
use crate::red::{Red, RedKind, Result};

pub use crate::objbuild::CENSUS_CRATES;

/// Attribute paths leg (1) counts (03 §6 (a)).
pub const COUNTED_ATTRS: [&str; 5] = ["no_mangle", "export_name", "link_section", "used", "linkage"];

/// What a finding counts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Counted {
    /// (a) `#[<name>]` for a name in [`COUNTED_ATTRS`].
    Attr(String),
    /// (b) a fn definition with an explicit ABI other than `"Rust"`; `None` = a bare `extern fn`.
    AbiFn(Option<String>),
    /// (c) `global_asm!`.
    GlobalAsm,
    /// (c) `naked_asm!`.
    NakedAsm,
    /// (c) `#[naked]`.
    NakedAttr,
}

impl fmt::Display for Counted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attr(a) => write!(f, "#[{a}]"),
            Self::AbiFn(Some(abi)) => write!(f, "extern {abi:?} fn"),
            Self::AbiFn(None) => f.write_str("extern fn (bare)"),
            Self::GlobalAsm => f.write_str("global_asm!"),
            Self::NakedAsm => f.write_str("naked_asm!"),
            Self::NakedAttr => f.write_str("#[naked]"),
        }
    }
}

/// Which mechanism saw a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Via {
    /// The `syn` walk of a parsed item.
    SynItem,
    /// M-a: a token sequence inside a macro body.
    MacroTokens,
    /// A string literal of a build script.
    BuildRsString,
    /// M-b: the expanded output of a `boyko_macros` fixture.
    Expanded,
}

/// One counted occurrence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// The file (repo-relative, `/` separators) or the fixture name.
    pub file: String,
    /// The item path the occurrence sits in (`module::Type::fn`, `macro_rules! name`, …).
    pub item: String,
    /// What was counted.
    pub kind: Counted,
    /// Which mechanism saw it.
    pub via: Via,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} | {} | {} | {:?}", self.file, self.item, self.kind, self.via)
    }
}

/// What a census read; every leg prints it, and a zero is RED.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Crates walked.
    pub crates: usize,
    /// `.rs` files parsed.
    pub files: usize,
    /// Items visited (items, impl items, trait items, foreign items, at every depth).
    pub items: usize,
    /// Macro bodies scanned by M-a (invocations and `macro_rules!` definitions).
    pub macro_bodies: usize,
    /// `macro_rules!` definitions among them.
    pub macro_rules: usize,
    /// Build-script string literals scanned.
    pub build_rs_strings: usize,
}

impl Stats {
    fn add(&mut self, o: &Self) {
        self.crates += o.crates;
        self.files += o.files;
        self.items += o.items;
        self.macro_bodies += o.macro_bodies;
        self.macro_rules += o.macro_rules;
        self.build_rs_strings += o.build_rs_strings;
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} crate(s), {} file(s), {} item(s), {} macro bod(ies) ({} macro_rules!), {} build.rs string(s)",
            self.crates, self.files, self.items, self.macro_bodies, self.macro_rules, self.build_rs_strings
        )
    }
}

/// The last identifier of a token path, with a raw-identifier prefix removed.
fn ident_text(t: &TokenTree) -> Option<String> {
    match t {
        TokenTree::Ident(i) => {
            let s = i.to_string();
            Some(s.strip_prefix("r#").map_or(s.clone(), str::to_owned))
        }
        _ => None,
    }
}

fn is_punct(t: Option<&TokenTree>, ch: char) -> bool {
    matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == ch)
}

fn paren_group(t: Option<&TokenTree>) -> Option<TokenStream> {
    match t {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => Some(g.stream()),
        _ => None,
    }
}

/// Splits `ts` at top-level commas.
fn split_commas(ts: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut out = vec![Vec::new()];
    for t in ts {
        if is_punct(Some(&t), ',') {
            out.push(Vec::new());
        } else if let Some(last) = out.last_mut() {
            last.push(t);
        }
    }
    out.retain(|v| !v.is_empty());
    out
}

/// Classifies the tokens INSIDE `#[ … ]` (a meta): a counted path, `unsafe(<meta>)`, or
/// `cfg_attr(<pred>, <meta>, …)` at any depth.
#[must_use]
pub fn classify_meta(tokens: &[TokenTree]) -> Vec<Counted> {
    let Some(first) = tokens.first().and_then(ident_text) else { return Vec::new() };
    if COUNTED_ATTRS.contains(&first.as_str()) {
        return vec![Counted::Attr(first)];
    }
    if first == "naked" {
        return vec![Counted::NakedAttr];
    }
    if first == "unsafe"
        && let Some(inner) = paren_group(tokens.get(1))
    {
        let v: Vec<TokenTree> = inner.into_iter().collect();
        return classify_meta(&v);
    }
    if first == "cfg_attr"
        && let Some(inner) = paren_group(tokens.get(1))
    {
        return split_commas(inner).iter().skip(1).flat_map(|m| classify_meta(m)).collect();
    }
    Vec::new()
}

/// `true` if the tokens at `i..` name a fn being DEFINED: an identifier, a quote interpolation
/// `#ident`, or a `macro_rules!` metavariable `$ident`. A fn-pointer type (`extern "C" fn(…)`)
/// has a parenthesised group there instead (critique O4).
fn defines_fn_name(v: &[TokenTree], i: usize) -> bool {
    let named = match v.get(i) {
        Some(TokenTree::Ident(_)) => true,
        Some(TokenTree::Punct(p)) if p.as_char() == '#' || p.as_char() == '$' => matches!(v.get(i + 1), Some(TokenTree::Ident(_))),
        _ => false,
    };
    // A named fn with a body (`{ … }`) is a definition; one ending in `;` is a trait method
    // declaration, which defines nothing (its impls are where the definitions are).
    named
        && v[i..]
            .iter()
            .find_map(|t| match t {
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => Some(true),
                TokenTree::Punct(p) if p.as_char() == ';' => Some(false),
                _ => None,
            })
            .unwrap_or(true)
}

fn string_literal_value(t: &TokenTree) -> Option<String> {
    let TokenTree::Literal(l) = t else { return None };
    syn::parse_str::<syn::LitStr>(&l.to_string()).ok().map(|s| s.value())
}

/// M-a: the counted forms among a macro body's tokens, recursively through every group.
///
/// Matched: `#[…]` and `#![…]` whose meta [`classify_meta`] counts; `extern "<abi>" fn <name>`
/// with `<abi>` ≠ `"Rust"`; `extern fn <name>`; `global_asm` / `naked_asm` followed by `!`.
#[must_use]
pub fn scan_tokens(ts: TokenStream) -> Vec<Counted> {
    let v: Vec<TokenTree> = ts.into_iter().collect();
    let mut out = Vec::new();
    for i in 0..v.len() {
        match &v[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let j = if is_punct(v.get(i + 1), '!') { i + 2 } else { i + 1 };
                if let Some(TokenTree::Group(g)) = v.get(j)
                    && g.delimiter() == Delimiter::Bracket
                {
                    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                    out.extend(classify_meta(&inner));
                }
            }
            TokenTree::Ident(id) => {
                let s = id.to_string();
                if s == "extern" {
                    match v.get(i + 1) {
                        Some(t @ TokenTree::Literal(_)) => {
                            if let Some(abi) = string_literal_value(t)
                                && matches!(v.get(i + 2).and_then(ident_text).as_deref(), Some("fn"))
                                && defines_fn_name(&v, i + 3)
                                && abi != "Rust"
                            {
                                out.push(Counted::AbiFn(Some(abi)));
                            }
                        }
                        Some(t) if ident_text(t).as_deref() == Some("fn") && defines_fn_name(&v, i + 2) => {
                            out.push(Counted::AbiFn(None));
                        }
                        _ => {}
                    }
                } else if (s == "global_asm" || s == "naked_asm") && is_punct(v.get(i + 1), '!') {
                    out.push(if s == "global_asm" { Counted::GlobalAsm } else { Counted::NakedAsm });
                }
            }
            _ => {}
        }
        if let TokenTree::Group(g) = &v[i] {
            out.extend(scan_tokens(g.stream()));
        }
    }
    out
}

/// The doc-line prefix of a modding-seam marker (05 §3, 03 §6 leg (5)).
pub const SEAM_MARKER: &str = "MOD-SEAM";

/// The trait name of the modding-seam token (05 §3.2; lands at D-S1(ii)).
pub const SEAM_TRAIT: &str = "ModSeam";

/// A `/// MOD-SEAM MS-xx: …` doc line on an item (leg 5).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Marker {
    /// The file.
    pub file: String,
    /// The marked item's path, as the walker spells it (`EcsMaster::add_component_by_id`).
    pub item: String,
    /// The MS id the line names (`MS-08`), or the whole line when it names none (a RED).
    pub ms: String,
    /// The item, or an impl enclosing it, has a generic parameter bounded by `ModSeam`.
    pub generic_over_seam: bool,
}

/// Leg (5)'s structural facts about the `ModSeam` trait in one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeamFacts {
    /// Doc markers.
    pub markers: Vec<Marker>,
    /// `impl … ModSeam for …` outside `#[cfg(test)]`, as (file, impl label).
    pub impls_outside_test: Vec<(String, String)>,
    /// A crate root's `pub use` naming `ModSeam`, as (file, use tree).
    pub root_reexports: Vec<(String, String)>,
    /// A crate root's `pub use …::*` (a glob the walker cannot resolve), as (file, use tree).
    pub root_globs: Vec<(String, String)>,
    /// Files that define `trait ModSeam`.
    pub trait_defs: Vec<String>,
}

/// The walker's state for one file.
struct Walker<'a> {
    file: &'a str,
    via_items: Via,
    path: Vec<String>,
    findings: Vec<Finding>,
    stats: Stats,
    seam: SeamFacts,
    /// One entry per enclosing impl: does its generics bound a parameter by `ModSeam`?
    seam_generic: Vec<bool>,
    /// Depth of enclosing `#[cfg(test)]` items.
    cfg_test: usize,
    /// The file is a crate root (`lib.rs`), so its top-level `pub use` re-exports at the root.
    crate_root: bool,
}

/// `true` if `generics` has a type parameter bounded by [`SEAM_TRAIT`], inline or in `where`.
fn bounds_seam(generics: &syn::Generics) -> bool {
    let names_seam = |b: &syn::TypeParamBound| matches!(b, syn::TypeParamBound::Trait(t) if t.path.segments.last().is_some_and(|s| s.ident == SEAM_TRAIT));
    generics.type_params().any(|p| p.bounds.iter().any(names_seam))
        || generics.where_clause.as_ref().is_some_and(|w| {
            w.predicates.iter().any(|p| matches!(p, syn::WherePredicate::Type(t) if t.bounds.iter().any(names_seam)))
        })
}

/// `true` if `attrs` holds `#[cfg(test)]` (or a `cfg` whose predicate mentions `test` at top level).
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") && matches!(&a.meta, syn::Meta::List(l) if l.tokens.to_string().trim() == "test")
    })
}

/// The MOD-SEAM lines among `attrs`' doc strings: the MS id each names, or the whole line.
fn seam_lines(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for a in attrs {
        if !a.path().is_ident("doc") {
            continue;
        }
        let syn::Meta::NameValue(nv) = &a.meta else { continue };
        let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value else { continue };
        for line in s.value().lines() {
            let Some(rest) = line.trim().strip_prefix(SEAM_MARKER) else { continue };
            let id: String = rest.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            out.push(if id.starts_with("MS-") { id } else { line.trim().to_owned() });
        }
    }
    out
}

fn use_tree_names(t: &syn::UseTree, prefix: &str, out: &mut Vec<(String, bool)>) {
    match t {
        syn::UseTree::Path(p) => use_tree_names(&p.tree, &format!("{prefix}{}::", p.ident), out),
        syn::UseTree::Name(n) => out.push((format!("{prefix}{}", n.ident), false)),
        syn::UseTree::Rename(r) => out.push((format!("{prefix}{} as {}", r.ident, r.rename), false)),
        syn::UseTree::Glob(_) => out.push((format!("{prefix}*"), true)),
        syn::UseTree::Group(g) => {
            for i in &g.items {
                use_tree_names(i, prefix, out);
            }
        }
    }
}

impl Walker<'_> {
    fn here(&self) -> String {
        if self.path.is_empty() { "<file>".to_owned() } else { self.path.join("::") }
    }

    /// Records the item's MOD-SEAM lines; `generics` is the item's own, if it has any.
    fn note_markers(&mut self, attrs: &[syn::Attribute], generics: Option<&syn::Generics>) {
        let lines = seam_lines(attrs);
        if lines.is_empty() {
            return;
        }
        let generic = generics.is_some_and(bounds_seam) || self.seam_generic.iter().any(|g| *g);
        for ms in lines {
            self.seam.markers.push(Marker { file: self.file.to_owned(), item: self.here(), ms, generic_over_seam: generic });
        }
    }

    fn push(&mut self, kind: Counted, via: Via) {
        self.findings.push(Finding { file: self.file.to_owned(), item: self.here(), kind, via });
    }

    fn check_abi(&mut self, abi: Option<&syn::Abi>) {
        let Some(abi) = abi else { return };
        match &abi.name {
            None => self.push(Counted::AbiFn(None), self.via_items),
            Some(name) if name.value() != "Rust" => self.push(Counted::AbiFn(Some(name.value())), self.via_items),
            Some(_) => {}
        }
    }

    fn scoped<F: FnOnce(&mut Self)>(&mut self, name: String, f: F) {
        self.path.push(name);
        f(self);
        self.path.pop();
    }
}

fn type_label(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map_or_else(|| "?".to_owned(), |s| s.ident.to_string()),
        other => other.to_token_stream().to_string(),
    }
}

impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_item(&mut self, i: &'ast syn::Item) {
        self.stats.items += 1;
        visit::visit_item(self, i);
    }

    fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
        self.stats.items += 1;
        visit::visit_impl_item(self, i);
    }

    fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
        self.stats.items += 1;
        visit::visit_trait_item(self, i);
    }

    fn visit_foreign_item(&mut self, i: &'ast syn::ForeignItem) {
        self.stats.items += 1;
        visit::visit_foreign_item(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let test = is_cfg_test(&i.attrs);
        self.cfg_test += usize::from(test);
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, None);
            visit::visit_item_mod(w, i);
        });
        self.cfg_test -= usize::from(test);
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        self.scoped(i.sig.ident.to_string(), |w| {
            w.check_abi(i.sig.abi.as_ref());
            w.note_markers(&i.attrs, Some(&i.sig.generics));
            visit::visit_item_fn(w, i);
        });
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.scoped(i.sig.ident.to_string(), |w| {
            w.check_abi(i.sig.abi.as_ref());
            w.note_markers(&i.attrs, Some(&i.sig.generics));
            visit::visit_impl_item_fn(w, i);
        });
    }

    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        self.scoped(i.sig.ident.to_string(), |w| {
            // A declaration without a body defines nothing; its impls are counted where they are.
            if i.default.is_some() {
                w.check_abi(i.sig.abi.as_ref());
            }
            w.note_markers(&i.attrs, Some(&i.sig.generics));
            visit::visit_trait_item_fn(w, i);
        });
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let label = match &i.trait_ {
            Some((_, path, _)) => {
                let t = path.segments.last().map_or_else(|| "?".to_owned(), |s| s.ident.to_string());
                if t == SEAM_TRAIT && self.cfg_test == 0 && !is_cfg_test(&i.attrs) {
                    self.seam.impls_outside_test.push((self.file.to_owned(), format!("impl {t} for {}", type_label(&i.self_ty))));
                }
                format!("<{} as {t}>", type_label(&i.self_ty))
            }
            None => type_label(&i.self_ty),
        };
        let test = is_cfg_test(&i.attrs);
        self.cfg_test += usize::from(test);
        self.seam_generic.push(bounds_seam(&i.generics));
        self.scoped(label, |w| {
            w.note_markers(&i.attrs, None);
            visit::visit_item_impl(w, i);
        });
        self.seam_generic.pop();
        self.cfg_test -= usize::from(test);
    }

    fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
        if i.ident == SEAM_TRAIT {
            self.seam.trait_defs.push(self.file.to_owned());
        }
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_trait(w, i);
        });
    }

    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_struct(w, i);
        });
    }

    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_enum(w, i);
        });
    }

    fn visit_item_union(&mut self, i: &'ast syn::ItemUnion) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_union(w, i);
        });
    }

    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, None);
            visit::visit_item_static(w, i);
        });
    }

    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_const(w, i);
        });
    }

    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        self.scoped(i.ident.to_string(), |w| {
            w.note_markers(&i.attrs, Some(&i.generics));
            visit::visit_item_type(w, i);
        });
    }

    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        if self.crate_root && self.path.is_empty() && matches!(i.vis, syn::Visibility::Public(_)) {
            let mut names = Vec::new();
            use_tree_names(&i.tree, "", &mut names);
            for (n, glob) in names {
                if glob {
                    self.seam.root_globs.push((self.file.to_owned(), n));
                } else if n.rsplit("::").next().is_some_and(|last| last.split(" as ").next() == Some(SEAM_TRAIT)) {
                    self.seam.root_reexports.push((self.file.to_owned(), n));
                }
            }
        }
        visit::visit_item_use(self, i);
    }

    fn visit_item_macro(&mut self, i: &'ast syn::ItemMacro) {
        match &i.ident {
            Some(name) => {
                self.stats.macro_rules += 1;
                self.scoped(format!("macro_rules! {name}"), |w| visit::visit_item_macro(w, i));
            }
            None => visit::visit_item_macro(self, i),
        }
    }

    fn visit_attribute(&mut self, a: &'ast syn::Attribute) {
        let tokens: Vec<TokenTree> = a.meta.to_token_stream().into_iter().collect();
        for c in classify_meta(&tokens) {
            self.push(c, self.via_items);
        }
        visit::visit_attribute(self, a);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.stats.macro_bodies += 1;
        if let Some(last) = m.path.segments.last() {
            let name = last.ident.to_string();
            if name == "global_asm" {
                self.push(Counted::GlobalAsm, self.via_items);
            } else if name == "naked_asm" {
                self.push(Counted::NakedAsm, self.via_items);
            }
        }
        let via = if self.via_items == Via::Expanded { Via::Expanded } else { Via::MacroTokens };
        for c in scan_tokens(m.tokens.clone()) {
            self.push(c, via);
        }
        visit::visit_macro(self, m);
    }
}

/// Runs the one visitor over a parsed file. `file` labels the findings; `via` is
/// [`Via::SynItem`] for source and [`Via::Expanded`] for a macro fixture's output.
#[must_use]
pub fn count_file(file: &str, parsed: &syn::File, via: Via) -> (Vec<Finding>, Stats) {
    let (f, s, _) = walk_file(file, parsed, via);
    (f, s)
}

/// [`count_file`] plus leg (5)'s seam facts. A file named `lib.rs` is treated as a crate root.
#[must_use]
pub fn walk_file(file: &str, parsed: &syn::File, via: Via) -> (Vec<Finding>, Stats, SeamFacts) {
    let mut w = Walker {
        file,
        via_items: via,
        path: Vec::new(),
        findings: Vec::new(),
        stats: Stats::default(),
        seam: SeamFacts::default(),
        seam_generic: Vec::new(),
        cfg_test: 0,
        crate_root: file.ends_with("/src/lib.rs") || file == "lib.rs",
    };
    w.visit_file(parsed);
    w.stats.files = 1;
    (w.findings, w.stats, w.seam)
}

/// The counted forms in text that does not tokenize: attribute-shaped (`#[…` / `#![…`, through
/// `unsafe(` and `cfg_attr(`), `extern "…" fn`, `extern fn`, `global_asm!`, `naked_asm!`.
/// Conservative on purpose: a false hit is a RED someone reads, a miss is a hole.
#[must_use]
pub fn scan_text(text: &str) -> Vec<Counted> {
    let mut out = Vec::new();
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_owned).collect()
    };
    let mut rest = text;
    while let Some(i) = rest.find('#') {
        let after = rest[i + 1..].trim_start_matches('!').trim_start();
        if let Some(body) = after.strip_prefix('[') {
            let end = body.find(']').unwrap_or(body.len());
            for w in words(&body[..end]) {
                if COUNTED_ATTRS.contains(&w.as_str()) {
                    out.push(Counted::Attr(w));
                } else if w == "naked" {
                    out.push(Counted::NakedAttr);
                }
            }
        }
        rest = &rest[i + 1..];
    }
    let ws = words(text);
    for (k, w) in ws.iter().enumerate() {
        match w.as_str() {
            "global_asm" => out.push(Counted::GlobalAsm),
            "naked_asm" => out.push(Counted::NakedAsm),
            "extern" if ws.get(k + 1).is_some_and(|n| n == "fn") => out.push(Counted::AbiFn(None)),
            _ => {}
        }
    }
    let mut rest = text;
    while let Some(i) = rest.find("extern") {
        let after = rest[i + 6..].trim_start();
        if let Some(q) = after.strip_prefix('"')
            && let Some(close) = q.find('"')
            && q[close + 1..].trim_start().starts_with("fn")
            && &q[..close] != "Rust"
        {
            out.push(Counted::AbiFn(Some(q[..close].to_owned())));
        }
        rest = &rest[i + 6..];
    }
    out
}

fn string_literals(ts: TokenStream, out: &mut Vec<String>) {
    for t in ts {
        match &t {
            TokenTree::Group(g) => string_literals(g.stream(), out),
            TokenTree::Literal(_) => {
                if let Some(v) = string_literal_value(&t) {
                    out.push(v);
                }
            }
            _ => {}
        }
    }
}

/// Every string literal of a build script, including those inside `format!` / `writeln!`
/// arguments: a literal that tokenizes is scanned with M-a, one that does not with [`scan_text`].
pub fn scan_build_rs(file: &str, src: &str) -> Result<(Vec<Finding>, Stats)> {
    let ts = TokenStream::from_str(src).map_err(|e| Red::new(RedKind::Malformed, format!("{file} does not tokenize: {e}")))?;
    let mut lits = Vec::new();
    string_literals(ts, &mut lits);
    let mut findings = Vec::new();
    for lit in &lits {
        let counted = match TokenStream::from_str(lit) {
            Ok(inner) => scan_tokens(inner),
            Err(_) => scan_text(lit),
        };
        for kind in counted {
            findings.push(Finding { file: file.to_owned(), item: format!("string literal {:?}", truncate(lit, 60)), kind, via: Via::BuildRsString });
        }
    }
    let stats = Stats { files: 1, build_rs_strings: lits.len(), ..Stats::default() };
    Ok((findings, stats))
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if t.len() < s.len() { format!("{t}…") } else { t }
}

/// `root`-relative path with `/` separators.
#[must_use]
pub fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Every `.rs` file under `dir`, sorted.
pub fn rs_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = std::fs::read_dir(&d).map_err(|e| Red::io(&d, &e))?;
        for e in rd {
            let e = e.map_err(|e| Red::io(&d, &e))?;
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Parses `path` as a Rust file; RED on an unreadable or unparseable file (a file the walk
/// cannot read is not a file with zero findings).
pub fn parse_path(path: &Path) -> Result<syn::File> {
    let src = std::fs::read_to_string(path).map_err(|e| Red::io(path, &e))?;
    syn::parse_file(&src).map_err(|e| Red::new(RedKind::Malformed, format!("{} does not parse: {e}", path.display())))
}

/// Leg (1): the census of the six crates under `root` — `src/**` with the one visitor, plus
/// each crate's `build.rs` string literals.
///
/// RED on a missing crate directory or a crate with zero files: an empty walk has no zero to
/// report.
pub fn census(root: &Path) -> Result<(Vec<Finding>, Stats, BTreeMap<String, Stats>)> {
    let mut all = Vec::new();
    let mut total = Stats::default();
    let mut per: BTreeMap<String, Stats> = BTreeMap::new();
    for c in CENSUS_CRATES {
        let dir = root.join("crates").join(c);
        let src = dir.join("src");
        if !src.is_dir() {
            return Err(Red::new(RedKind::EmptyCensus, format!("census crate {c} has no src directory at {}", src.display())));
        }
        let mut s = Stats { crates: 1, ..Stats::default() };
        for f in rs_files(&src)? {
            let label = rel(root, &f);
            let parsed = parse_path(&f)?;
            let (fs, st) = count_file(&label, &parsed, Via::SynItem);
            all.extend(fs);
            s.add(&st);
        }
        let build = dir.join("build.rs");
        if build.is_file() {
            let text = std::fs::read_to_string(&build).map_err(|e| Red::io(&build, &e))?;
            let (fs, st) = scan_build_rs(&rel(root, &build), &text)?;
            all.extend(fs);
            s.build_rs_strings += st.build_rs_strings;
        }
        if s.files == 0 {
            return Err(Red::new(RedKind::EmptyCensus, format!("census crate {c}: zero .rs files walked")));
        }
        total.add(&s);
        per.insert(c.to_owned(), s);
    }
    all.sort();
    Ok((all, total, per))
}

/// One owner-signed allowlist entry of leg (1).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllowEntry {
    /// The finding's file.
    pub file: String,
    /// The finding's item path.
    pub item: String,
    /// The counted form, as [`Counted`]'s `Display` prints it.
    pub kind: String,
    /// Why the owner accepts it.
    pub reason: String,
}

/// Parses the allowlist: `[[allow]]` tables of `file`, `item`, `kind`, `reason` string keys, `#`
/// comments. Anything else is RED — a line this parser skipped would be an entry nobody reviewed.
pub fn parse_allowlist(text: &str) -> Result<Vec<AllowEntry>> {
    let mut out: Vec<AllowEntry> = Vec::new();
    let mut cur: Option<BTreeMap<String, String>> = None;
    let finish = |cur: &mut Option<BTreeMap<String, String>>, out: &mut Vec<AllowEntry>| -> Result<()> {
        if let Some(mut m) = cur.take() {
            let mut take = |k: &str| {
                m.remove(k).ok_or_else(|| Red::new(RedKind::Malformed, format!("allowlist entry lacks `{k}`")))
            };
            out.push(AllowEntry { file: take("file")?, item: take("item")?, kind: take("kind")?, reason: take("reason")? });
            if !m.is_empty() {
                return Err(Red::new(RedKind::Malformed, format!("allowlist entry has unknown keys {:?}", m.keys())));
            }
        }
        Ok(())
    };
    for (n, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l == "[[allow]]" {
            finish(&mut cur, &mut out)?;
            cur = Some(BTreeMap::new());
            continue;
        }
        let (k, v) = l.split_once('=').ok_or_else(|| Red::new(RedKind::Malformed, format!("allowlist line {}: `{l}`", n + 1)))?;
        let v = syn::parse_str::<syn::LitStr>(v.trim())
            .map_err(|_| Red::new(RedKind::Malformed, format!("allowlist line {}: the value is not a string literal", n + 1)))?
            .value();
        let Some(m) = cur.as_mut() else {
            return Err(Red::new(RedKind::Malformed, format!("allowlist line {}: a key outside an [[allow]] table", n + 1)));
        };
        m.insert(k.trim().to_owned(), v);
    }
    finish(&mut cur, &mut out)?;
    out.sort();
    Ok(out)
}

/// A census crate's direct dependency, as leg (1)'s dependency pin records it (critique W6).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DepRow {
    /// The census crate (directory name).
    pub krate: String,
    /// The dependency's package name.
    pub dep: String,
    /// `normal`, `build` or `dev` (one row per kind).
    pub kind: String,
    /// `proc-macro` if the dependency has a proc-macro target, else `lib`.
    pub target: String,
}

impl fmt::Display for DepRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\t{}\t{}\t{}", self.krate, self.dep, self.kind, self.target)
    }
}

/// Every direct dependency of every census crate, from `cargo metadata`'s resolve graph.
///
/// Why: a dependency's macro emits its tokens into the census crate at expansion, where the
/// source walk cannot see them (a `linkme`/`inventory`/`ctor`-style registry puts a
/// `#[link_section]` static into the crate that invokes it). The pin makes a NEW macro source
/// RED until someone has read what it emits.
pub fn census_crate_deps(root: &Path, cargo: &std::ffi::OsStr) -> Result<Vec<DepRow>> {
    let out = std::process::Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo metadata: {e}")))?;
    if !out.status.success() {
        return Err(Red::new(RedKind::BuildFailed, format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let meta = crate::json::parse(&String::from_utf8_lossy(&out.stdout))?;
    let mut name_of: BTreeMap<&str, &str> = BTreeMap::new();
    let mut is_pm: BTreeMap<&str, bool> = BTreeMap::new();
    let mut census_ids: BTreeMap<&str, String> = BTreeMap::new();
    for p in meta.get("packages").map(Json::items).unwrap_or(&[]) {
        let (Some(id), Some(name)) = (p.get("id").and_then(Json::as_str), p.get("name").and_then(Json::as_str)) else { continue };
        name_of.insert(id, name);
        let pm = p
            .get("targets")
            .map(Json::items)
            .unwrap_or(&[])
            .iter()
            .any(|t| t.get("kind").map(Json::items).unwrap_or(&[]).iter().any(|k| k.as_str() == Some("proc-macro")));
        is_pm.insert(id, pm);
        if let Some(mp) = p.get("manifest_path").and_then(Json::as_str) {
            let dir = Path::new(mp).parent().and_then(Path::file_name).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            let under_crates = Path::new(mp).parent().and_then(Path::parent).and_then(Path::file_name).is_some_and(|d| d == "crates");
            if under_crates && CENSUS_CRATES.contains(&dir.as_str()) {
                census_ids.insert(id, dir);
            }
        }
    }
    if census_ids.len() != CENSUS_CRATES.len() {
        return Err(Red::new(
            RedKind::EmptyCensus,
            format!("cargo metadata names {} of the {} census crates: {:?}", census_ids.len(), CENSUS_CRATES.len(), census_ids.values()),
        ));
    }
    let mut rows = Vec::new();
    let nodes = meta.get("resolve").and_then(|r| r.get("nodes")).map(Json::items).unwrap_or(&[]);
    for n in nodes {
        let Some(id) = n.get("id").and_then(Json::as_str) else { continue };
        let Some(krate) = census_ids.get(id) else { continue };
        for d in n.get("deps").map(Json::items).unwrap_or(&[]) {
            let Some(pkg) = d.get("pkg").and_then(Json::as_str) else { continue };
            let dep = name_of.get(pkg).copied().unwrap_or(pkg).to_owned();
            let target = if is_pm.get(pkg).copied().unwrap_or(false) { "proc-macro" } else { "lib" }.to_owned();
            for k in d.get("dep_kinds").map(Json::items).unwrap_or(&[]) {
                let kind = k.get("kind").and_then(Json::as_str).unwrap_or("normal").to_owned();
                rows.push(DepRow { krate: krate.clone(), dep: dep.clone(), kind, target: target.clone() });
            }
        }
    }
    rows.sort();
    rows.dedup();
    if rows.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, "cargo metadata's resolve graph gave the census crates no dependency at all"));
    }
    Ok(rows)
}

/// An identifier-shaped string literal in a proc-macro parser module (M-b's key self-census).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyLiteral {
    /// The literal's value.
    pub text: String,
    /// The item path it sits in.
    pub item: String,
    /// `matches!` when it came from that macro's tokens, `code` otherwise.
    pub site: &'static str,
}

fn ident_shaped(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

struct KeyWalker {
    path: Vec<String>,
    out: Vec<KeyLiteral>,
}

impl KeyWalker {
    fn push(&mut self, text: String, site: &'static str) {
        if ident_shaped(&text) {
            self.out.push(KeyLiteral { text, item: self.path.join("::"), site });
        }
    }
}

impl<'ast> Visit<'ast> for KeyWalker {
    fn visit_item(&mut self, i: &'ast syn::Item) {
        let attrs: &[syn::Attribute] = match i {
            syn::Item::Mod(m) => &m.attrs,
            syn::Item::Fn(f) => &f.attrs,
            syn::Item::Impl(x) => &x.attrs,
            syn::Item::Const(c) => &c.attrs,
            syn::Item::Static(s) => &s.attrs,
            _ => &[],
        };
        if is_cfg_test(attrs) {
            return;
        }
        let name = match i {
            syn::Item::Mod(m) => Some(m.ident.to_string()),
            syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
            syn::Item::Impl(x) => Some(type_label(&x.self_ty)),
            syn::Item::Const(c) => Some(c.ident.to_string()),
            syn::Item::Static(s) => Some(s.ident.to_string()),
            _ => None,
        };
        if let Some(n) = &name {
            self.path.push(n.clone());
        }
        visit::visit_item(self, i);
        if name.is_some() {
            self.path.pop();
        }
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        if is_cfg_test(&i.attrs) {
            return;
        }
        self.path.push(i.sig.ident.to_string());
        visit::visit_impl_item_fn(self, i);
        self.path.pop();
    }

    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        self.push(l.value(), "code");
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // Every other macro's tokens are emitted code (`quote!`) or messages (`format!`,
        // `panic!`); `matches!` is a comparison, so its string literals are keys.
        if m.path.segments.last().is_some_and(|s| s.ident == "matches") {
            let mut lits = Vec::new();
            string_literals(m.tokens.clone(), &mut lits);
            for l in lits {
                self.push(l, "matches!");
            }
        }
    }
}

/// Every identifier-shaped string literal of `file`'s non-test code: in expressions, patterns
/// and constants, and inside `matches!` — the candidate keys of a parser (M-b's self-census,
/// `boyko_macros::ug15_corpus::keys_are_the_parsers_own`). Literals inside any other macro
/// (`quote!` templates, `format!` messages) are emitted text, not keys, and are not listed.
#[must_use]
pub fn key_literals(file: &syn::File) -> Vec<KeyLiteral> {
    let mut w = KeyWalker { path: Vec::new(), out: Vec::new() };
    w.visit_file(file);
    w.out.sort();
    w.out
}

/// What an expanded fixture contains, for M-b's anchor checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outline {
    /// `(trait last segment, self type label)` of every trait impl, at any depth.
    pub trait_impls: Vec<(String, String)>,
    /// The last path segment of every macro invocation, at any depth.
    pub macros: Vec<String>,
    /// The identifier of every named item, at any depth.
    pub items: Vec<String>,
}

struct OutlineWalker(Outline);

impl<'ast> Visit<'ast> for OutlineWalker {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if let Some((_, p, _)) = &i.trait_ {
            let t = p.segments.last().map_or_else(String::new, |s| s.ident.to_string());
            self.0.trait_impls.push((t, type_label(&i.self_ty)));
        }
        visit::visit_item_impl(self, i);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if let Some(s) = m.path.segments.last() {
            self.0.macros.push(s.ident.to_string());
        }
        visit::visit_macro(self, m);
    }

    fn visit_item(&mut self, i: &'ast syn::Item) {
        let id = match i {
            syn::Item::Fn(f) => Some(&f.sig.ident),
            syn::Item::Struct(s) => Some(&s.ident),
            syn::Item::Enum(e) => Some(&e.ident),
            syn::Item::Mod(m) => Some(&m.ident),
            syn::Item::Static(s) => Some(&s.ident),
            syn::Item::Const(c) => Some(&c.ident),
            syn::Item::Type(t) => Some(&t.ident),
            syn::Item::Trait(t) => Some(&t.ident),
            _ => None,
        };
        if let Some(id) = id {
            self.0.items.push(id.to_string());
        }
        visit::visit_item(self, i);
    }
}

/// The outline of an expanded file.
#[must_use]
pub fn outline(file: &syn::File) -> Outline {
    let mut w = OutlineWalker(Outline::default());
    w.visit_file(file);
    w.0
}

/// The input-shape arms a proc-macro module branches on (critique W5): `Data::Struct`,
/// `Data::Enum`, `Data::Union`, `Fields::Named`, `Fields::Unnamed`, `Fields::Unit` wherever the
/// path appears (patterns, expressions), and `generics` for a `.generics` field read or a
/// `split_for_impl` call.
#[must_use]
pub fn shape_arms(file: &syn::File) -> std::collections::BTreeSet<String> {
    struct W(std::collections::BTreeSet<String>);
    impl<'ast> Visit<'ast> for W {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.len() >= 2 {
                let (outer, arm) = (&segs[segs.len() - 2], &segs[segs.len() - 1]);
                let known = matches!((outer.as_str(), arm.as_str()), ("Data", "Struct" | "Enum" | "Union") | ("Fields", "Named" | "Unnamed" | "Unit"));
                if known {
                    self.0.insert(format!("{outer}::{arm}"));
                }
            }
            visit::visit_path(self, p);
        }

        fn visit_expr_field(&mut self, e: &'ast syn::ExprField) {
            if matches!(&e.member, syn::Member::Named(n) if n == "generics") {
                self.0.insert("generics".to_owned());
            }
            visit::visit_expr_field(self, e);
        }

        fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
            if e.method == "split_for_impl" {
                self.0.insert("generics".to_owned());
            }
            visit::visit_expr_method_call(self, e);
        }
    }
    let mut w = W(std::collections::BTreeSet::new());
    w.visit_file(file);
    w.0
}

/// The first segment after `crate::` of every path and `use` tree in `file`: the sibling
/// modules it reaches.
#[must_use]
pub fn crate_refs(file: &syn::File) -> std::collections::BTreeSet<String> {
    struct W(std::collections::BTreeSet<String>);
    impl<'ast> Visit<'ast> for W {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            let mut it = p.segments.iter();
            if it.next().is_some_and(|s| s.ident == "crate")
                && let Some(s) = it.next()
            {
                self.0.insert(s.ident.to_string());
            }
            visit::visit_path(self, p);
        }

        fn visit_use_path(&mut self, u: &'ast syn::UsePath) {
            if u.ident == "crate" {
                let mut names = Vec::new();
                use_tree_names(&u.tree, "", &mut names);
                for (n, _) in names {
                    if let Some(first) = n.split("::").next() {
                        self.0.insert(first.split(" as ").next().unwrap_or(first).to_owned());
                    }
                }
                return;
            }
            visit::visit_use_path(self, u);
        }
    }
    let mut w = W(std::collections::BTreeSet::new());
    w.visit_file(file);
    w.0
}
