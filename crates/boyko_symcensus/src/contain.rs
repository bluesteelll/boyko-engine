//! Sensitivity map (c), the containment map (03 §6 "(2) Sensitivity map"): for each leg-(3)
//! type, the workspace types it holds BY VALUE, transitively, from a `syn` walk of the census
//! crates (the leg-(1) walker's file set).
//!
//! **Descends** through arrays, slices, tuples, parentheses, and every generic type argument of
//! any path type — `Option`, `Cell`, `UnsafeCell`, `OnceLock`, `ManuallyDrop`, `MaybeUninit`,
//! `CachePadded`, and ALSO every type in neither list (`Mutex`, `RefCell`, `LazyLock`,
//! `Result`, …), so the map over-approximates, which is the conservative direction (critique O6).
//!
//! **Stops** at references, raw pointers, fn pointers, trait objects, and the path types of
//! [`STOP_AT`]: they hold their pointee behind an indirection, so the pointee's layout does not
//! move the holder's field displacements.
//!
//! **Names resolve by last segment** against every struct / enum / union / type alias defined
//! in the census crates. An ambiguous name includes every candidate definition.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use syn::visit::{self, Visit};

use crate::red::{Red, RedKind, Result};
use crate::source::{self, CENSUS_CRATES};

/// Path types the walk does not descend through (they hold their argument behind a pointer).
pub const STOP_AT: [&str; 6] = ["NonNull", "Box", "Vec", "Arc", "Rc", "PhantomData"];

/// The leg-(3) types the map is taken for.
pub const ROOTS: [&str; 3] = ["EcsMaster", "ComponentPool", "Scope"];

/// Rows the plan records at `d552be05` (03 §6); a capture without them is RED.
pub const KNOWN_ROWS: [(&str, &str); 2] = [("EcsMaster", "CommandQueue"), ("EcsMaster", "QueryStateCache")];

/// A type definition: where, and the field / aliased types it holds.
#[derive(Clone)]
struct Def {
    file: String,
    types: Vec<syn::Type>,
}

#[derive(Default)]
struct Defs {
    by_name: BTreeMap<String, Vec<Def>>,
    file: String,
}

impl<'ast> Visit<'ast> for Defs {
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        let types = i.fields.iter().map(|f| f.ty.clone()).collect();
        self.by_name.entry(i.ident.to_string()).or_default().push(Def { file: self.file.clone(), types });
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        let types = i.variants.iter().flat_map(|v| v.fields.iter().map(|f| f.ty.clone())).collect();
        self.by_name.entry(i.ident.to_string()).or_default().push(Def { file: self.file.clone(), types });
        visit::visit_item_enum(self, i);
    }

    fn visit_item_union(&mut self, i: &'ast syn::ItemUnion) {
        let types = i.fields.named.iter().map(|f| f.ty.clone()).collect();
        self.by_name.entry(i.ident.to_string()).or_default().push(Def { file: self.file.clone(), types });
        visit::visit_item_union(self, i);
    }

    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        self.by_name.entry(i.ident.to_string()).or_default().push(Def { file: self.file.clone(), types: vec![(*i.ty).clone()] });
        visit::visit_item_type(self, i);
    }
}

/// The last-segment names `ty` holds by value (before resolution).
fn held_names(ty: &syn::Type, out: &mut BTreeSet<String>) {
    match ty {
        syn::Type::Array(a) => held_names(&a.elem, out),
        syn::Type::Slice(s) => held_names(&s.elem, out),
        syn::Type::Tuple(t) => t.elems.iter().for_each(|e| held_names(e, out)),
        syn::Type::Paren(p) => held_names(&p.elem, out),
        syn::Type::Group(g) => held_names(&g.elem, out),
        syn::Type::Path(p) => {
            if p.qself.is_some() {
                return;
            }
            let Some(last) = p.path.segments.last() else { return };
            let name = last.ident.to_string();
            if STOP_AT.contains(&name.as_str()) {
                return;
            }
            out.insert(name);
            if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                for a in &args.args {
                    if let syn::GenericArgument::Type(t) = a {
                        held_names(t, out);
                    }
                }
            }
        }
        // References, raw pointers, fn pointers, trait objects, `impl Trait`, macros, `!`, `_`.
        _ => {}
    }
}

/// The containment map of the census crates under `root`.
#[derive(Clone, Debug, Default)]
pub struct Containment {
    /// `(root type, contained type) → the defining files of the contained type`.
    pub rows: BTreeMap<(String, String), BTreeSet<String>>,
    /// Files walked.
    pub files: usize,
    /// Type definitions seen.
    pub defs: usize,
}

/// Walks the census crates and computes the transitive by-value containment of each of [`ROOTS`].
pub fn capture(root: &Path) -> Result<Containment> {
    let mut defs = Defs::default();
    let mut files = 0usize;
    for c in CENSUS_CRATES {
        for f in source::rs_files(&root.join("crates").join(c).join("src"))? {
            defs.file = source::rel(root, &f);
            defs.visit_file(&source::parse_path(&f)?);
            files += 1;
        }
    }
    let mut out = Containment { files, defs: defs.by_name.values().map(Vec::len).sum(), ..Containment::default() };
    for r in ROOTS {
        if !defs.by_name.contains_key(r) {
            return Err(Red::new(RedKind::SymbolAbsent, format!("map (c): the leg-(3) type {r} has no definition in the census crates")));
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue = vec![r.to_owned()];
        while let Some(t) = queue.pop() {
            for d in defs.by_name.get(&t).map(Vec::as_slice).unwrap_or(&[]) {
                let mut names = BTreeSet::new();
                for ty in &d.types {
                    held_names(ty, &mut names);
                }
                for n in names {
                    let Some(nd) = defs.by_name.get(&n) else { continue };
                    if n != r && seen.insert(n.clone()) {
                        queue.push(n.clone());
                    }
                    let files: BTreeSet<String> = nd.iter().map(|d| d.file.clone()).collect();
                    if n != r {
                        out.rows.entry((r.to_owned(), n)).or_default().extend(files);
                    }
                }
            }
        }
    }
    for (a, b) in KNOWN_ROWS {
        if !out.rows.contains_key(&(a.to_owned(), b.to_owned())) {
            return Err(Red::new(RedKind::Mismatch, format!("map (c) lacks the plan's known row {a} ⊃ {b} (03 §6)")));
        }
    }
    Ok(out)
}

/// The TSV text of a map: `root\tcontained\tdefining files (comma-joined)`.
#[must_use]
pub fn to_tsv(c: &Containment) -> String {
    let mut s = String::from("# UG-15 sensitivity map (c): containment, by value, transitive (03 §6). root\tcontained\tdefined in\n");
    for ((r, t), files) in &c.rows {
        let f: Vec<&str> = files.iter().map(String::as_str).collect();
        let _ = writeln!(s, "{r}\t{t}\t{}", f.join(","));
    }
    s
}
