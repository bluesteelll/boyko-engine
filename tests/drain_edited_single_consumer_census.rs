//! **AD-16 (render-wave RD5): every `Assets<T>` edited set has exactly one production drainer.**
//!
//! `Assets::drain_edited` hands each row marked since the last drain to its callback and CLEARS
//! the marks. A GPU mirror keyed on the edited set (the material stager, the particle effect bake)
//! therefore depends on being its table's only drainer: a second system draining the same table
//! would take marks the first one never sees, and the first mirror would silently miss edits. No
//! runtime check can see that — both drainers are correct alone — so this census reads the source.
//!
//! # What it asserts
//!
//! Every `drain_edited` use in the production source (`crates/*/src/**`, `#[cfg(test)]` items
//! skipped) — a method call, a path (`Assets::<T>::drain_edited`), or the identifier inside a
//! macro invocation — is a row of [`EXPECTED`], and every row is found. Red when:
//!
//! - a use is not in [`EXPECTED`] (a second drainer, or a drain moved into a helper);
//! - an [`EXPECTED`] row is not found (the drainer it names stopped draining);
//! - a row's enclosing fn signature does not name `Assets<T>` for that row's `T` (a drain moved
//!   into a generic `fn helper<T>(a: &mut Assets<T>)`, or a row naming the wrong table);
//! - two [`EXPECTED`] rows name the same `T` (one table, two drainers);
//! - one fn holds more than one use;
//! - a file containing `drain_edited` does not parse;
//! - anti-vacuity: the walk read fewer than [`MIN_FILES`] files, or the kernel no longer defines
//!   `pub fn drain_edited(` in [`DEFINING_FILE`].
//!
//! Scope is the production source, not `crates/*/tests`: AD-16 is about systems that ship, and an
//! integration test may drain a table it built for itself. Test-only code inside `src` is skipped
//! twice over: inline `#[cfg(test)]` items by the visitor, and the files of an out-of-line
//! `#[cfg(test)] mod name;` (`name.rs`, and everything under `name/`) by the walk. The kernel file
//! is walked like any other — its definition is not a use and its model tests are `#[cfg(test)]`
//! — so a kernel-side wrapper that drains under another name is a red, not a blind spot.
//!
//! # Why this lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree, and the package links no engine crate (`Cargo.toml`'s dev-dependency note).

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};
use syn::{GenericArgument, ImplItem, Item, PathArguments, Signature, TraitItem, Type};

/// The production drainers: `(file, enclosing fn, the T of the Assets<T> it drains)`.
const EXPECTED: &[(&str, &str, &str)] = &[
    ("crates/boyko_render/src/material_upload.rs", "stage_from", "Material"),
    ("crates/boyko_render/src/particle_system.rs", "particle_pack_effects", "ParticleEffect"),
];

/// The file that defines `Assets::drain_edited`.
const DEFINING_FILE: &str = "crates/boyko_ecs/src/ecs/core/asset/assets.rs";

/// Anti-vacuity floor on the `.rs` files the walk reads under `crates/*/src`.
const MIN_FILES: usize = 500;

/// One `drain_edited` use.
struct Site {
    file: String,
    /// The enclosing fn's name, or `<no fn>` at item level.
    func: String,
    /// The enclosing fn's signature, if any.
    sig: Option<Signature>,
    how: &'static str,
}

/// Every `.rs` file under `dir`, sorted, skipping `target` directories.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target") {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The files of every out-of-line `#[cfg(test)] mod name;` declared in `file` (whose text is
/// `text`): the module's `name.rs` and its `name/` directory, both as absolute paths.
fn test_only_module_paths(file: &Path, text: &str) -> Vec<PathBuf> {
    let dir = file.parent().expect("invariant: a source file has a parent directory");
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let base =
        if matches!(stem, "mod" | "lib" | "main") { dir.to_path_buf() } else { dir.join(stem) };
    let mut out = Vec::new();
    let mut pending_cfg_test = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "#[cfg(test)]" {
            pending_cfg_test = true;
            continue;
        }
        if pending_cfg_test && t.starts_with("#[") {
            continue;
        }
        if pending_cfg_test {
            let decl =
                t.strip_prefix("pub(crate) ").or_else(|| t.strip_prefix("pub ")).unwrap_or(t);
            if let Some(name) = decl.strip_prefix("mod ").and_then(|r| r.strip_suffix(';')) {
                out.push(base.join(format!("{name}.rs")));
                out.push(base.join(name));
            }
        }
        pending_cfg_test = false;
    }
    out
}

/// `true` iff `attrs` carry exactly `#[cfg(test)]`.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && matches!(&a.meta, syn::Meta::List(list) if list.tokens.to_string().trim() == "test")
    })
}

/// `true` iff `ts` holds the identifier `drain_edited` at any depth.
fn tokens_name_drain_edited(ts: TokenStream) -> bool {
    ts.into_iter().any(|tt| match tt {
        TokenTree::Ident(id) => id == "drain_edited",
        TokenTree::Group(g) => tokens_name_drain_edited(g.stream()),
        _ => false,
    })
}

/// Collects the `drain_edited` uses of one file, tracking the enclosing fn.
struct SiteVisitor<'a> {
    file: &'a str,
    enclosing: Vec<Signature>,
    sites: &'a mut Vec<Site>,
}

impl SiteVisitor<'_> {
    fn record(&mut self, how: &'static str) {
        let sig = self.enclosing.last().cloned();
        let func = sig.as_ref().map_or_else(|| "<no fn>".to_owned(), |s| s.ident.to_string());
        self.sites.push(Site { file: self.file.to_owned(), func, sig, how });
    }
}

impl<'ast> Visit<'ast> for SiteVisitor<'_> {
    fn visit_item(&mut self, i: &'ast Item) {
        let attrs: &[syn::Attribute] = match i {
            Item::Fn(f) => &f.attrs,
            Item::Impl(imp) => &imp.attrs,
            Item::Mod(m) => &m.attrs,
            Item::Trait(t) => &t.attrs,
            Item::Const(c) => &c.attrs,
            Item::Static(s) => &s.attrs,
            Item::Macro(m) => &m.attrs,
            _ => &[],
        };
        if !is_cfg_test(attrs) {
            visit::visit_item(self, i);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.enclosing.push(f.sig.clone());
        visit::visit_item_fn(self, f);
        self.enclosing.pop();
    }

    fn visit_impl_item(&mut self, i: &'ast ImplItem) {
        let skip = match i {
            ImplItem::Fn(f) => is_cfg_test(&f.attrs),
            ImplItem::Const(c) => is_cfg_test(&c.attrs),
            _ => false,
        };
        if !skip {
            visit::visit_impl_item(self, i);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.enclosing.push(f.sig.clone());
        visit::visit_impl_item_fn(self, f);
        self.enclosing.pop();
    }

    fn visit_trait_item(&mut self, i: &'ast TraitItem) {
        let skip = matches!(i, TraitItem::Fn(f) if is_cfg_test(&f.attrs));
        if !skip {
            visit::visit_trait_item(self, i);
        }
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        self.enclosing.push(f.sig.clone());
        visit::visit_trait_item_fn(self, f);
        self.enclosing.pop();
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        if m.method == "drain_edited" {
            self.record("method call");
        }
        visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        if p.path.segments.last().is_some_and(|s| s.ident == "drain_edited") {
            self.record("path");
        }
        visit::visit_expr_path(self, p);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // syn does not parse a macro's body, so the identifier is looked for in its tokens.
        if tokens_name_drain_edited(m.tokens.clone()) {
            self.record("inside a macro");
        }
        visit::visit_macro(self, m);
    }
}

/// `true` iff `sig` names `Assets<..>` whose type argument's last path segment is `t` (so
/// `Assets<crate::Material>` counts, and `Assets<T>` for a generic `T` does not).
fn names_assets_of(sig: &Signature, t: &str) -> bool {
    struct Finder<'t> {
        t: &'t str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_path_segment(&mut self, seg: &'ast syn::PathSegment) {
            if seg.ident == "Assets"
                && let PathArguments::AngleBracketed(args) = &seg.arguments
            {
                let t = self.t;
                let hit = args.args.iter().any(|a| {
                    matches!(a, GenericArgument::Type(Type::Path(p))
                        if p.path.segments.last().is_some_and(|s| s.ident == t))
                });
                self.found |= hit;
            }
            visit::visit_path_segment(self, seg);
        }
    }
    let mut finder = Finder { t, found: false };
    finder.visit_signature(sig);
    finder.found
}

#[test]
fn every_assets_edited_set_has_exactly_one_drainer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let mut crate_dirs: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("invariant: crates/ is readable")
        .map(|e| e.expect("invariant: entry").path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    crate_dirs.sort();
    for src in &crate_dirs {
        rust_files(src, &mut files);
    }

    let texts: Vec<(&PathBuf, String)> = files
        .iter()
        .map(|p| (p, fs::read_to_string(p).expect("invariant: UTF-8 source")))
        .collect();
    let test_only: Vec<PathBuf> =
        texts.iter().flat_map(|(p, text)| test_only_module_paths(p, text)).collect();

    let mut bad = Vec::new();
    let mut sites = Vec::new();
    let mut prefiltered = 0usize;
    let mut skipped_test_files = 0usize;
    let mut defining_seen = false;
    for (path, text) in &texts {
        let rel = path
            .strip_prefix(root)
            .expect("invariant: the walk stays under the repository root")
            .to_string_lossy()
            .replace('\\', "/");
        if test_only.iter().any(|t| path.starts_with(t)) {
            skipped_test_files += 1;
            continue;
        }
        if rel == DEFINING_FILE {
            defining_seen = text.contains("pub fn drain_edited(");
        }
        if !text.contains("drain_edited") {
            continue;
        }
        prefiltered += 1;
        match syn::parse_file(text) {
            Ok(ast) => {
                SiteVisitor { file: &rel, enclosing: Vec::new(), sites: &mut sites }
                    .visit_file(&ast);
            }
            Err(e) => bad.push(format!("{rel} mentions drain_edited and does not parse: {e}")),
        }
    }

    println!(
        "walked {} files under crates/*/src ({skipped_test_files} in out-of-line #[cfg(test)] \
         modules skipped); {prefiltered} mention drain_edited; {} use(s):",
        files.len(),
        sites.len()
    );
    for s in &sites {
        println!("  {}::{} ({})", s.file, s.func, s.how);
    }

    if files.len() < MIN_FILES {
        bad.push(format!("anti-vacuity: the walk read {} files (floor {MIN_FILES})", files.len()));
    }
    if !defining_seen {
        bad.push(format!("anti-vacuity: {DEFINING_FILE} no longer defines `pub fn drain_edited(`"));
    }
    for (i, (_, _, t)) in EXPECTED.iter().enumerate() {
        if EXPECTED[..i].iter().any(|(_, _, u)| u == t) {
            bad.push(format!("EXPECTED names two drainers of Assets<{t}>"));
        }
    }
    for (i, s) in sites.iter().enumerate() {
        if sites[..i].iter().any(|o| o.file == s.file && o.func == s.func) {
            bad.push(format!("{}::{} holds more than one drain_edited use", s.file, s.func));
        }
        match EXPECTED.iter().find(|(f, func, _)| *f == s.file && *func == s.func) {
            None => bad.push(format!(
                "unlisted drain_edited use: {}::{} ({}) - a second drainer of an edited set, or \
                 a drain moved out of its listed fn",
                s.file, s.func, s.how
            )),
            Some((_, _, t)) => {
                if !s.sig.as_ref().is_some_and(|sig| names_assets_of(sig, t)) {
                    bad.push(format!(
                        "{}::{} drains, but its signature does not name Assets<{t}>",
                        s.file, s.func
                    ));
                }
            }
        }
    }
    for (file, func, t) in EXPECTED {
        if !sites.iter().any(|s| s.file == *file && s.func == *func) {
            bad.push(format!("expected drainer missing: {file}::{func} (Assets<{t}>)"));
        }
    }

    println!("violations ({}):", bad.len());
    for b in &bad {
        println!("  {b}");
    }
    assert!(bad.is_empty(), "{} drain_edited census violation(s):\n{}", bad.len(), bad.join("\n"));
}
