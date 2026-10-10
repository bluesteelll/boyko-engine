//! UG-02's scanner, comparator and data readers.
//!
//! Included by `tests/runtime_data_ledger_gate.rs` with `#[path]`, and by the ledger recount's
//! out-of-tree line dump (the only caller that can print line numbers: `Span::start` exists only
//! under proc-macro2's `span-locations`, which this workspace does not enable, so the line
//! provider is a parameter and the gate passes one that answers `None`).
//!
//! # What a site is
//!
//! One declaration or constructor in non-test code of a scanned crate that owns or creates heap
//! memory. The site key is line-free (`kind|item_path|container`), so a line shift is not a diff,
//! and the comparison against the ledger is a multiset over `(file, key)`: two rows may share a
//! key, and then two sites must exist. Syntax, not text: a text scanner misses tuple variants,
//! aliases, `cfg(not(test))`, hand-wrapped generics and every head but `Vec<` (allocator design
//! P6, W4), and each of those is an ordinary AST node here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Block, Expr, Fields, FnArg, GenericArgument, ImplItem, Item, Macro, Meta, Pat,
    PathArguments, ReturnType, Signature, Stmt, Token, TraitItem, Type, UseTree,
};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Vocabularies that live in gate code (critique W4: a data edit alone must not be able to lower a
// pin, so the things that can only grow are anchored here and the data file may only add).
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Where a heap head comes from. `Own` heads are existence-checked against the scanned crates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Origin {
    /// `std` / `alloc`.
    Std,
    /// A third-party crate's heap-holding type.
    ThirdParty,
    /// One of this workspace's own heap-holding types (the ledger's double representation).
    Own,
}

/// The baseline heap heads. `ug02-vocab.tsv` may ADD heads; removing one of these is a gate-code
/// edit, visible in review (W4).
pub const BASELINE_HEADS: &[(&str, Origin)] = &[
    ("Vec", Origin::Std),
    ("VecDeque", Origin::Std),
    ("String", Origin::Std),
    ("Box", Origin::Std),
    ("Rc", Origin::Std),
    ("Arc", Origin::Std),
    ("BTreeMap", Origin::Std),
    ("BTreeSet", Origin::Std),
    ("HashMap", Origin::Std),
    ("HashSet", Origin::Std),
    ("BinaryHeap", Origin::Std),
    ("LinkedList", Origin::Std),
    ("Cow", Origin::Std),
    ("PathBuf", Origin::Std),
    ("OsString", Origin::Std),
    ("CString", Origin::Std),
    ("BufWriter", Origin::Std),
    ("BufReader", Origin::Std),
    ("FixedBitSet", Origin::ThirdParty),
    ("ArrayQueue", Origin::ThirdParty),
    ("SegQueue", Origin::ThirdParty),
    ("Injector", Origin::ThirdParty),
    ("Worker", Origin::ThirdParty),
    ("Stealer", Origin::ThirdParty),
    ("SmallList4", Origin::Own),
    ("SparseMap", Origin::Own),
    ("LiveBitmap", Origin::Own),
    ("VisitedSet", Origin::Own),
    ("SparseSlotMap", Origin::Own),
    ("UiParseReport", Origin::Own),
];

/// The crates that must stay scanned in the mode given here (W4: flipping a crate to unscanned
/// or pending would lower every count in it, so the baseline modes live in gate code; the data
/// file may add crates, never demote these).
pub const BASELINE_SCANNED: &[(&str, Mode)] = &[
    ("crates/boyko_ecs", Mode::Full),
    ("crates/boyko_memory", Mode::Full),
    ("crates/boyko_utils", Mode::Full),
    ("crates/boyko_threadpool", Mode::Full),
    ("crates/boyko_log", Mode::Full),
    ("crates/boyko_diag", Mode::Full),
    ("crates/boyko_math", Mode::Full),
    ("crates/boyko_sdf_math", Mode::Full),
    ("crates/boyko_shaderdsl", Mode::Full),
    ("crates/boyko_physics", Mode::Full),
    ("crates/boyko_scene", Mode::Full),
    ("crates/boyko_input", Mode::Full),
    ("crates/boyko_serialize", Mode::Full),
    ("crates/boyko_image", Mode::Full),
    ("crates/boyko_fontbake", Mode::Full),
    ("crates/boyko_rhi", Mode::Full),
    ("crates/boyko_rhi_vulkan", Mode::Full),
    ("crates/boyko_render", Mode::Full),
    ("crates/boyko_ui", Mode::Full),
    ("crates/boyko_reflect", Mode::Full),
    ("crates/boyko_app", Mode::Full),
    ("crates/boyko_demo", Mode::Full),
    (".", Mode::Full),
    ("crates/boyko_macros", Mode::Emitted),
];

/// Plan rulings that keep a heap site BY DESIGN, so a row may leave the pin as
/// `out-of-scope:ruled` (W4: the tag is tied to this closed list, not to any `U-\d+`).
/// U-5: the per-lane Chase-Lev deques stay crossbeam in v1. U-19: `boyko_diag` depends on
/// nothing, so its lane cell stays a const-initialised `thread_local!`.
pub const KEEPING_RULINGS: &[&str] = &["U-5", "U-19"];

/// `ecs_form` values (the ledger's closed list; `new-kernel-feature:<name>` and
/// `out-of-scope:<tag>` are checked separately).
const FORMS: &[&str] = &[
    "component",
    "dense-component",
    "enable-state",
    "relation",
    "event",
    "resource-column",
    "system-scratch",
    "scope-arena",
    "kernel-internal",
    "diagnostics",
];
const OUT_OF_SCOPE_TAGS: &[&str] = &[
    "os-owned",
    "driver-owned",
    "compile-time",
    "test-only",
    "third-party",
    "ruled",
];
const CLASSES: &[&str] = &["K", "E", "R", "F", "S", "B", "X", "D", "T", "C"];
const KINDS: &[&str] = &["local", "field", "return", "param", "static"];
const GROWTHS: &[&str] = &["once", "highwater", "unknown", "perframe"];
const ADDR_CACHED: &[&str] = &["yes", "no"];
/// The thirteen group files.
pub const GROUPS: &[&str] = &[
    "ecs-storage",
    "ecs-schedule",
    "ecs-services",
    "pool-utils-log",
    "physics-scene-math",
    "render",
    "rhi",
    "ui-input",
    "app-demo",
    "codec-tools",
    "macros-aether",
    "ui-lane",
    "reflect-lane",
];
const OWNING_ENTITIES: &[&str] = &[
    "body",
    "soft-body",
    "particle",
    "mesh",
    "material",
    "asset",
    "light",
    "widget",
    "text-run",
    "window",
    "observer",
    "prefab",
    "system",
    "schedule",
    "pool",
    "relation-endpoint",
    "none",
];
/// Forms for which `owning_entity = none` is legal (the ledger's "How to read a row").
const NONE_ENTITY_FORMS: &[&str] = &[
    "resource-column",
    "kernel-internal",
    "diagnostics",
    "scope-arena",
];
/// Container prefixes of kernel storage (M5): rows for their form decision only.
const M5_CONTAINERS: &[&str] = &["other:ScratchColumn", "other:VmColumn"];

/// The TSV header, in order: the twelve rev-4 columns plus rev 5's `group`, `shape`, `site`.
pub const TSV_HEADER: &[&str] = &[
    "crate",
    "file",
    "owner",
    "kind",
    "container",
    "elem",
    "class",
    "ecs_form",
    "owning_entity",
    "growth",
    "addr_cached",
    "destination",
    "group",
    "shape",
    "site",
];

/// The pins file's header.
pub const PINS_HEADER: &[&str] = &[
    "rung",
    "kind",
    "active_std_heap_rows",
    "total_active_rows",
    "semantic_rows",
    "out_of_scope_rows",
    "physics_pool_scope",
    "physics_active_pool",
    "floors",
    "prev",
    "reason",
];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Sites
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A line provider: `None` in the gate (no `span-locations`), the start line in the dump.
pub type LineOf<'a> = &'a dyn Fn(Span) -> Option<u32>;

/// The syntactic shape of a site (the TSV `shape` column).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Shape {
    /// A field of a struct, enum variant or union.
    Field,
    /// A `static` / `static mut`.
    Static,
    /// A `static` inside `thread_local!`.
    Tls,
    /// An owning head in a return type of a fn with a body.
    Ret,
    /// An owning or `&mut` head in a parameter of a fn with a body.
    Param,
    /// A typed `let` (absorbs the constructor that forms its initialiser).
    Let,
    /// A constructor call not absorbed by a declaration.
    Ctor,
    /// A raw `std::alloc` call.
    Raw,
    /// A head found by token scan in a macro body or unparsed invocation.
    Macro,
    /// A head found by token scan in a `quote!` template of an emitted-mode crate.
    Emitted,
}

impl Shape {
    /// The TSV spelling.
    pub const fn name(self) -> &'static str {
        match self {
            Shape::Field => "field",
            Shape::Static => "static",
            Shape::Tls => "tls",
            Shape::Ret => "ret",
            Shape::Param => "param",
            Shape::Let => "let",
            Shape::Ctor => "ctor",
            Shape::Raw => "raw",
            Shape::Macro => "macro",
            Shape::Emitted => "emitted",
        }
    }

    /// The ledger `kind` this shape reports.
    pub const fn kind(self) -> &'static str {
        match self {
            Shape::Field => "field",
            Shape::Static | Shape::Tls => "static",
            Shape::Ret => "return",
            Shape::Param => "param",
            Shape::Let | Shape::Ctor | Shape::Raw | Shape::Macro | Shape::Emitted => "local",
        }
    }

    /// The first component of the site key: the kind for syntactic shapes, the shape itself for
    /// the two token-scanned ones (their item path is a macro name or a file, not a declaration).
    const fn key_head(self) -> &'static str {
        match self {
            Shape::Macro => "macro",
            Shape::Emitted => "emitted",
            other => other.kind(),
        }
    }

    /// Every shape, for vocabulary checks and receipts.
    pub const ALL: [Shape; 10] = [
        Shape::Field,
        Shape::Static,
        Shape::Tls,
        Shape::Ret,
        Shape::Param,
        Shape::Let,
        Shape::Ctor,
        Shape::Raw,
        Shape::Macro,
        Shape::Emitted,
    ];

    fn from_name(s: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|sh| sh.name() == s)
    }
}

/// One scanned site.
#[derive(Clone, Debug)]
pub struct Site {
    /// Crate directory (`crates/boyko_ecs`, or `.` for the root package).
    pub crate_dir: String,
    /// Repository-relative file, forward slashes.
    pub file: String,
    /// The shape.
    pub shape: Shape,
    /// `[mod::]*(Type::|<Type as Trait>::)?fn[::nested]*`, `Type::field`, `static NAME`, …
    pub item_path: String,
    /// The canonical container key.
    pub container: String,
    /// The site's token text, at most 120 chars.
    pub text: String,
    /// The site's own start line (dump only).
    pub line: Option<u32>,
    /// The enclosing statement's start line (dump only; the ledger rowed some constructors at
    /// the statement's first line).
    pub stmt_line: Option<u32>,
}

impl Site {
    /// The line-free site key.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.shape.key_head(),
            self.item_path,
            self.container
        )
    }
}

/// A physics-slice hit: `pool.scope(` or `try_with_active_pool(` in `boyko_physics` non-test code.
#[derive(Clone, Debug)]
pub struct SliceHit {
    /// Repository-relative file.
    pub file: String,
    /// Enclosing item path.
    pub item_path: String,
    /// `scope` or `try_with_active_pool`.
    pub which: &'static str,
    /// Token text.
    pub text: String,
    /// Line (dump only).
    pub line: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// TSV reading
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A parsed TSV: its header and its data rows with their 1-based file line numbers.
pub struct Tsv {
    /// Header cells.
    pub header: Vec<String>,
    /// `(line, cells)`.
    pub rows: Vec<(usize, Vec<String>)>,
}

/// Reads a TSV. `\r` is stripped (git holds these files `i/lf w/crlf`). Lines that are empty or
/// start with `#` are comments. Every data row must have the header's cell count.
pub fn read_tsv(text: &str, what: &str) -> Result<Tsv, String> {
    let mut header: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<String> = line.split('\t').map(str::to_owned).collect();
        match &header {
            None => header = Some(cells),
            Some(h) => {
                if cells.len() != h.len() {
                    return Err(format!(
                        "{what}:{}: {} cells, header has {}",
                        i + 1,
                        cells.len(),
                        h.len()
                    ));
                }
                rows.push((i + 1, cells));
            }
        }
    }
    let header = header.ok_or_else(|| format!("{what}: no header line"))?;
    Ok(Tsv { header, rows })
}

fn expect_header(t: &Tsv, want: &[&str], what: &str) -> Result<(), String> {
    if t.header.iter().map(String::as_str).ne(want.iter().copied()) {
        return Err(format!(
            "{what}: header is {:?}, expected {:?}",
            t.header, want
        ));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Declarations (`docs/memory/ledger/ug02-crates.tsv`)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// How a crate is scanned.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Mode {
    /// Every shape.
    Full,
    /// Only `quote!` templates (the code it emits runs in every user crate).
    Emitted,
    /// Not scanned, for a reason the gate checks mechanically (no scanned crate depends on it).
    Unscanned,
    /// Declared ahead of its landing; legal only while the path is not a workspace member (W5).
    Pending,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        match s {
            "full" => Some(Mode::Full),
            "emitted" => Some(Mode::Emitted),
            "unscanned" => Some(Mode::Unscanned),
            "pending" => Some(Mode::Pending),
            _ => None,
        }
    }

    /// The TSV spelling.
    pub const fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Emitted => "emitted",
            Mode::Unscanned => "unscanned",
            Mode::Pending => "pending",
        }
    }
}

/// `ug02-crates.tsv`: columns `kind path value reason`.
#[derive(Default)]
pub struct Decls {
    /// `crate` rows: path → (mode, reason).
    pub crates: BTreeMap<String, (Mode, String)>,
    /// `include` rows: (file, argument token text).
    pub includes: BTreeSet<(String, String)>,
    /// `unreached` rows: files under a scanned `src/` that the module walk does not reach.
    pub unreached: BTreeSet<String>,
    /// `third-party` rows: (crate path, dependency name) → reason.
    pub third_party: BTreeMap<(String, String), String>,
}

/// Parses `ug02-crates.tsv`.
pub fn parse_decls(text: &str) -> Result<Decls, String> {
    let t = read_tsv(text, "ug02-crates.tsv")?;
    expect_header(&t, &["kind", "path", "value", "reason"], "ug02-crates.tsv")?;
    let mut d = Decls::default();
    for (line, c) in &t.rows {
        if c[3].trim().is_empty() {
            return Err(format!("ug02-crates.tsv:{line}: empty reason"));
        }
        match c[0].as_str() {
            "crate" => {
                let mode = Mode::parse(&c[2])
                    .ok_or_else(|| format!("ug02-crates.tsv:{line}: unknown mode `{}`", c[2]))?;
                if d.crates
                    .insert(c[1].clone(), (mode, c[3].clone()))
                    .is_some()
                {
                    return Err(format!(
                        "ug02-crates.tsv:{line}: crate `{}` declared twice",
                        c[1]
                    ));
                }
            }
            "include" => {
                d.includes.insert((c[1].clone(), c[2].clone()));
            }
            "unreached" => {
                d.unreached.insert(c[1].clone());
            }
            "third-party" => {
                d.third_party
                    .insert((c[1].clone(), c[2].clone()), c[3].clone());
            }
            other => return Err(format!("ug02-crates.tsv:{line}: unknown kind `{other}`")),
        }
    }
    Ok(d)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Heap-head vocabulary (`ug02-vocab.tsv`, additions to BASELINE_HEADS)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The resolved head vocabulary.
pub struct Vocab {
    /// Head name → origin.
    pub heads: BTreeMap<String, Origin>,
}

impl Vocab {
    /// The baseline alone (fixtures).
    pub fn baseline() -> Vocab {
        Vocab {
            heads: BASELINE_HEADS
                .iter()
                .map(|(n, o)| ((*n).to_owned(), *o))
                .collect(),
        }
    }

    /// The baseline plus `ug02-vocab.tsv` (columns `head origin reason`). A data row may only add.
    pub fn with_additions(text: &str) -> Result<Vocab, String> {
        let t = read_tsv(text, "ug02-vocab.tsv")?;
        expect_header(&t, &["head", "origin", "reason"], "ug02-vocab.tsv")?;
        let mut v = Vocab::baseline();
        for (line, c) in &t.rows {
            let origin = match c[1].as_str() {
                "std" => Origin::Std,
                "third-party" => Origin::ThirdParty,
                "own" => Origin::Own,
                other => return Err(format!("ug02-vocab.tsv:{line}: unknown origin `{other}`")),
            };
            if c[2].trim().is_empty() {
                return Err(format!("ug02-vocab.tsv:{line}: empty reason"));
            }
            if v.heads.insert(c[0].clone(), origin).is_some() {
                return Err(format!(
                    "ug02-vocab.tsv:{line}: head `{}` already declared",
                    c[0]
                ));
            }
        }
        Ok(v)
    }

    fn is_head(&self, name: &str) -> bool {
        self.heads.contains_key(name)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// cfg evaluation: `test`, `miri`, `loom`, `doc`, `doctest` are false; everything else unknown.
// An item is excluded iff its predicate is PROVABLY false (so `cfg(not(test))` is counted).
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tri {
    True,
    False,
    Unknown,
}

fn cfg_value(meta: &Meta) -> Tri {
    match meta {
        Meta::Path(p) => {
            if ["test", "miri", "loom", "doc", "doctest"]
                .iter()
                .any(|n| p.is_ident(n))
            {
                Tri::False
            } else {
                Tri::Unknown
            }
        }
        Meta::NameValue(_) => Tri::Unknown,
        Meta::List(l) => {
            let Ok(args) = l.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return Tri::Unknown;
            };
            let vals: Vec<Tri> = args.iter().map(cfg_value).collect();
            if l.path.is_ident("all") {
                if vals.contains(&Tri::False) {
                    Tri::False
                } else if vals.iter().all(|v| *v == Tri::True) {
                    Tri::True
                } else {
                    Tri::Unknown
                }
            } else if l.path.is_ident("any") {
                if vals.contains(&Tri::True) {
                    Tri::True
                } else if vals.iter().all(|v| *v == Tri::False) {
                    Tri::False
                } else {
                    Tri::Unknown
                }
            } else if l.path.is_ident("not") && vals.len() == 1 {
                match vals[0] {
                    Tri::True => Tri::False,
                    Tri::False => Tri::True,
                    Tri::Unknown => Tri::Unknown,
                }
            } else {
                Tri::Unknown
            }
        }
    }
}

/// True iff some `#[cfg(..)]` on the node is provably false, or the node is a `#[test]` /
/// `#[bench]` fn.
fn excluded(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("test") || a.path().is_ident("bench") {
            return true;
        }
        if !a.path().is_ident("cfg") {
            return false;
        }
        match a.parse_args::<Meta>() {
            Ok(m) => cfg_value(&m) == Tri::False,
            Err(_) => false,
        }
    })
}

fn expr_attrs(e: &Expr) -> &[Attribute] {
    match e {
        Expr::Array(x) => &x.attrs,
        Expr::Assign(x) => &x.attrs,
        Expr::Async(x) => &x.attrs,
        Expr::Await(x) => &x.attrs,
        Expr::Binary(x) => &x.attrs,
        Expr::Block(x) => &x.attrs,
        Expr::Break(x) => &x.attrs,
        Expr::Call(x) => &x.attrs,
        Expr::Cast(x) => &x.attrs,
        Expr::Closure(x) => &x.attrs,
        Expr::Const(x) => &x.attrs,
        Expr::Continue(x) => &x.attrs,
        Expr::Field(x) => &x.attrs,
        Expr::ForLoop(x) => &x.attrs,
        Expr::Group(x) => &x.attrs,
        Expr::If(x) => &x.attrs,
        Expr::Index(x) => &x.attrs,
        Expr::Let(x) => &x.attrs,
        Expr::Lit(x) => &x.attrs,
        Expr::Loop(x) => &x.attrs,
        Expr::Macro(x) => &x.attrs,
        Expr::Match(x) => &x.attrs,
        Expr::MethodCall(x) => &x.attrs,
        Expr::Paren(x) => &x.attrs,
        Expr::Path(x) => &x.attrs,
        Expr::Range(x) => &x.attrs,
        Expr::Reference(x) => &x.attrs,
        Expr::Repeat(x) => &x.attrs,
        Expr::Return(x) => &x.attrs,
        Expr::Struct(x) => &x.attrs,
        Expr::Try(x) => &x.attrs,
        Expr::TryBlock(x) => &x.attrs,
        Expr::Tuple(x) => &x.attrs,
        Expr::Unary(x) => &x.attrs,
        Expr::Unsafe(x) => &x.attrs,
        Expr::While(x) => &x.attrs,
        Expr::Yield(x) => &x.attrs,
        _ => &[],
    }
}

fn item_attrs(i: &Item) -> &[Attribute] {
    match i {
        Item::Const(x) => &x.attrs,
        Item::Enum(x) => &x.attrs,
        Item::ExternCrate(x) => &x.attrs,
        Item::Fn(x) => &x.attrs,
        Item::ForeignMod(x) => &x.attrs,
        Item::Impl(x) => &x.attrs,
        Item::Macro(x) => &x.attrs,
        Item::Mod(x) => &x.attrs,
        Item::Static(x) => &x.attrs,
        Item::Struct(x) => &x.attrs,
        Item::Trait(x) => &x.attrs,
        Item::TraitAlias(x) => &x.attrs,
        Item::Type(x) => &x.attrs,
        Item::Union(x) => &x.attrs,
        Item::Use(x) => &x.attrs,
        _ => &[],
    }
}

fn path_attr(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|a| {
        if !a.path().is_ident("path") {
            return None;
        }
        if let Meta::NameValue(nv) = &a.meta
            && let Expr::Lit(l) = &nv.value
            && let syn::Lit::Str(s) = &l.lit
        {
            return Some(s.value());
        }
        None
    })
}

/// A `#[cfg_attr(.., path = ..)]` would move a module file under a condition the walk cannot
/// decide; it is refused rather than guessed (critique W3).
fn has_cfg_attr_path(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg_attr")
            && a.meta
                .to_token_stream()
                .to_string()
                .split_whitespace()
                .any(|t| t == "path")
    })
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// `use` maps: local name → full path segments (renames resolve to the original name).
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Local name → full path of the `use`d item.
pub type UseMap = BTreeMap<String, Vec<String>>;

fn collect_uses_tree(prefix: &mut Vec<String>, tree: &UseTree, out: &mut UseMap) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_uses_tree(prefix, &p.tree, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let name = n.ident.to_string();
            if name == "self" {
                if let Some(last) = prefix.last() {
                    out.insert(last.clone(), prefix.clone());
                }
            } else {
                let mut full = prefix.clone();
                full.push(name.clone());
                out.insert(name, full);
            }
        }
        UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.insert(r.rename.to_string(), full);
        }
        UseTree::Glob(_) => {}
        UseTree::Group(g) => {
            for t in &g.items {
                collect_uses_tree(prefix, t, out);
            }
        }
    }
}

struct UseCollector<'m> {
    out: &'m mut UseMap,
}

impl<'ast> Visit<'ast> for UseCollector<'_> {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if excluded(&u.attrs) {
            return;
        }
        let mut prefix = Vec::new();
        collect_uses_tree(&mut prefix, &u.tree, self.out);
    }

    fn visit_item(&mut self, i: &'ast Item) {
        if excluded(item_attrs(i)) {
            return;
        }
        visit::visit_item(self, i);
    }
}

fn file_uses(f: &syn::File) -> UseMap {
    let mut out = UseMap::new();
    UseCollector { out: &mut out }.visit_file(f);
    out
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The module walk
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One parsed, reached file.
pub struct SrcFile {
    /// Repository-relative path.
    pub rel: String,
    /// The AST.
    pub ast: syn::File,
    /// Its `use` map.
    pub uses: UseMap,
}

/// What the walk of one crate produced.
#[derive(Default)]
pub struct CrateWalk {
    /// Reached files, in walk order.
    pub files: Vec<SrcFile>,
    /// Files excluded by a provably-false `cfg` on their `mod` declaration, and the directories
    /// under them.
    pub excluded_files: BTreeSet<String>,
    /// Directory prefixes excluded with them.
    pub excluded_dirs: BTreeSet<String>,
    /// Files the walk reached that do not parse (each is a PARSE failure).
    pub unparsed: BTreeSet<String>,
}

fn rel_of(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    let s = r.to_string_lossy().replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn crate_src(root: &Path, crate_dir: &str) -> PathBuf {
    if crate_dir == "." {
        root.join("src")
    } else {
        root.join(crate_dir).join("src")
    }
}

fn crate_roots(root: &Path, crate_dir: &str) -> Vec<PathBuf> {
    let src = crate_src(root, crate_dir);
    let mut roots = Vec::new();
    for n in ["lib.rs", "main.rs"] {
        let p = src.join(n);
        if p.is_file() {
            roots.push(p);
        }
    }
    let bin = src.join("bin");
    if let Ok(rd) = fs::read_dir(&bin) {
        let mut ents: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        ents.sort();
        for p in ents {
            if p.is_file() && p.extension().is_some_and(|x| x == "rs") {
                roots.push(p);
            } else if p.is_dir() && p.join("main.rs").is_file() {
                roots.push(p.join("main.rs"));
            }
        }
    }
    roots
}

struct Walker<'a> {
    root: &'a Path,
    failures: &'a mut Vec<String>,
    out: CrateWalk,
    seen: BTreeSet<PathBuf>,
}

impl Walker<'_> {
    fn load(&mut self, p: &Path, mod_rs: bool) {
        if !self.seen.insert(p.to_path_buf()) {
            return;
        }
        let rel = rel_of(self.root, p);
        let text = match fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                self.failures.push(format!("WALK  {rel}  cannot read: {e}"));
                return;
            }
        };
        let ast = match syn::parse_file(&text) {
            Ok(a) => a,
            Err(e) => {
                self.failures.push(format!("PARSE  {rel}  {e}"));
                // Reached, but unparsed: the PARSE line is its failure, not an ORPHAN one too.
                self.out.unparsed.insert(rel);
                return;
            }
        };
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let children = if mod_rs {
            dir.clone()
        } else {
            dir.join(
                p.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        };
        let mut pending: Vec<(PathBuf, bool, bool)> = Vec::new();
        self.mods(&ast.items, &dir, &children, true, &rel, &mut pending);
        let uses = file_uses(&ast);
        self.out.files.push(SrcFile { rel, ast, uses });
        for (path, is_mod_rs, is_excluded) in pending {
            if is_excluded {
                self.exclude(&path, is_mod_rs);
            } else {
                self.load(&path, is_mod_rs);
            }
        }
    }

    fn exclude(&mut self, p: &Path, mod_rs: bool) {
        self.out.excluded_files.insert(rel_of(self.root, p));
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let children = if mod_rs {
            dir
        } else {
            dir.join(
                p.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        };
        self.out.excluded_dirs.insert(rel_of(self.root, &children));
    }

    /// Resolves the `mod x;` declarations of one item list. `top` is true outside inline module
    /// blocks, where a `#[path]` is relative to the file's own directory.
    fn mods(
        &mut self,
        items: &[Item],
        file_dir: &Path,
        children: &Path,
        top: bool,
        rel: &str,
        pending: &mut Vec<(PathBuf, bool, bool)>,
    ) {
        for item in items {
            let Item::Mod(m) = item else { continue };
            let off = excluded(&m.attrs);
            if has_cfg_attr_path(&m.attrs) {
                self.failures.push(format!(
                    "WALK  {rel}  `mod {}` carries a cfg_attr(.., path = ..) the walk cannot decide",
                    m.ident
                ));
                continue;
            }
            match &m.content {
                Some((_, inner)) => {
                    // An excluded inline module's nested `mod x;` files are excluded with it.
                    let sub = children.join(m.ident.to_string());
                    if off {
                        self.out.excluded_dirs.insert(rel_of(self.root, &sub));
                        continue;
                    }
                    self.mods(inner, file_dir, &sub, false, rel, pending);
                }
                None => {
                    let (path, is_mod_rs) = if let Some(p) = path_attr(&m.attrs) {
                        let base = if top { file_dir } else { children };
                        (base.join(p), true)
                    } else {
                        let a = children.join(format!("{}.rs", m.ident));
                        let b = children.join(m.ident.to_string()).join("mod.rs");
                        if a.is_file() {
                            (a, false)
                        } else if b.is_file() {
                            (b, true)
                        } else {
                            if !off {
                                self.failures.push(format!(
                                    "WALK  {rel}  `mod {};` has no file ({} or {})",
                                    m.ident,
                                    rel_of(self.root, &a),
                                    rel_of(self.root, &b)
                                ));
                            }
                            continue;
                        }
                    };
                    if !off && !path.is_file() {
                        self.failures.push(format!(
                            "WALK  {rel}  `mod {};` path {} does not exist",
                            m.ident,
                            rel_of(self.root, &path)
                        ));
                        continue;
                    }
                    pending.push((path, is_mod_rs, off));
                }
            }
        }
    }
}

/// Walks one crate's module tree from its `src/` roots.
pub fn walk_crate(root: &Path, crate_dir: &str, failures: &mut Vec<String>) -> CrateWalk {
    let mut w = Walker {
        root,
        failures,
        out: CrateWalk::default(),
        seen: BTreeSet::new(),
    };
    for r in crate_roots(root, crate_dir) {
        w.load(&r, true);
    }
    w.out
}

fn all_rs_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut ents: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    ents.sort();
    for p in ents {
        if p.is_dir() {
            all_rs_under(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Crate-level tables: aliases and struct field types (for alias expansion and `.clone()`).
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct AliasDef {
    params: Vec<String>,
    ty: Type,
    file_idx: usize,
}

/// A type with the file whose `use` map resolves its names.
#[derive(Clone)]
struct Typed {
    ty: Type,
    file_idx: Option<usize>,
}

#[derive(Default)]
struct Tables {
    aliases: BTreeMap<String, Vec<AliasDef>>,
    /// Struct name → (field name, type, file index).
    structs: BTreeMap<String, Vec<(String, Type, usize)>>,
    /// Every type name defined (struct / enum / union / alias / trait), for own-head checks.
    defined: BTreeSet<String>,
    /// Free fn name → return types (a name defined twice is kept only if the types agree).
    fns: BTreeMap<String, Vec<Typed>>,
    /// (impl self type name, method) → return types, `Self` already replaced.
    methods: BTreeMap<(String, String), Vec<Typed>>,
    /// `thread_local!` static name → declared type.
    tls: BTreeMap<String, Typed>,
    /// Every module name declared in the crate (a path that starts with one is crate-local).
    mods: BTreeSet<String>,
}

/// One scanned crate's aliases, kept for the whole scan so that a qualified path or a `use` of
/// another scanned crate's alias resolves (review W5).
struct CrateAliases {
    /// The name code uses for the crate: `[lib] name`, else the package name with `-` → `_`,
    /// else the directory name.
    lib: String,
    aliases: BTreeMap<String, Vec<AliasDef>>,
    /// Per-file `use` maps, indexed by [`AliasDef::file_idx`].
    uses: Vec<UseMap>,
    mods: BTreeSet<String>,
}

fn crate_lib_name(root: &Path, crate_dir: &str) -> String {
    let manifest = if crate_dir == "." {
        root.join("Cargo.toml")
    } else {
        root.join(crate_dir).join("Cargo.toml")
    };
    let text = fs::read_to_string(manifest).unwrap_or_default();
    manifest_table_name(&text, "[lib]")
        .or_else(|| manifest_package_name(&text))
        .map(|n| n.replace('-', "_"))
        .unwrap_or_else(|| crate_dir.rsplit('/').next().unwrap_or(crate_dir).to_owned())
}

struct TableCollector<'t> {
    t: &'t mut Tables,
    file_idx: usize,
    self_ty: Option<Type>,
}

fn replace_self(ty: &Type, self_ty: Option<&Type>) -> Type {
    match (ty, self_ty) {
        (Type::Path(p), Some(s)) if p.qself.is_none() && p.path.is_ident("Self") => s.clone(),
        _ => ty.clone(),
    }
}

impl TableCollector<'_> {
    fn ret(&self, sig: &Signature) -> Option<Typed> {
        match &sig.output {
            ReturnType::Type(_, t) => Some(Typed {
                ty: replace_self(t, self.self_ty.as_ref()),
                file_idx: Some(self.file_idx),
            }),
            ReturnType::Default => None,
        }
    }
}

impl<'ast> Visit<'ast> for TableCollector<'_> {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if excluded(&i.attrs) {
            return;
        }
        let saved = self.self_ty.replace((*i.self_ty).clone());
        if let Some(name) = Visitor::type_name(&i.self_ty) {
            for it in &i.items {
                if let ImplItem::Fn(f) = it
                    && !excluded(&f.attrs)
                    && let Some(r) = self.ret(&f.sig)
                {
                    self.t
                        .methods
                        .entry((name.clone(), f.sig.ident.to_string()))
                        .or_default()
                        .push(r);
                }
            }
        }
        self.self_ty = saved;
    }

    fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
        if excluded(&m.attrs) || !m.mac.path.is_ident("thread_local") {
            return;
        }
        if let Ok(body) = m.mac.parse_body::<TlsBody>() {
            for d in body.0 {
                self.t.tls.insert(
                    d.ident.to_string(),
                    Typed {
                        ty: d.ty,
                        file_idx: Some(self.file_idx),
                    },
                );
            }
        }
    }

    fn visit_item(&mut self, i: &'ast Item) {
        if excluded(item_attrs(i)) {
            return;
        }
        match i {
            Item::Fn(f) => {
                if let Some(r) = self.ret(&f.sig) {
                    self.t
                        .fns
                        .entry(f.sig.ident.to_string())
                        .or_default()
                        .push(r);
                }
            }
            Item::Type(ty) => {
                let params = ty
                    .generics
                    .params
                    .iter()
                    .filter_map(|p| match p {
                        syn::GenericParam::Type(tp) => Some(tp.ident.to_string()),
                        _ => None,
                    })
                    .collect();
                self.t.defined.insert(ty.ident.to_string());
                self.t
                    .aliases
                    .entry(ty.ident.to_string())
                    .or_default()
                    .push(AliasDef {
                        params,
                        ty: (*ty.ty).clone(),
                        file_idx: self.file_idx,
                    });
            }
            Item::Struct(s) => {
                self.t.defined.insert(s.ident.to_string());
                let entry = self.t.structs.entry(s.ident.to_string()).or_default();
                if let Fields::Named(n) = &s.fields {
                    for f in &n.named {
                        if let Some(id) = &f.ident {
                            entry.push((id.to_string(), f.ty.clone(), self.file_idx));
                        }
                    }
                }
            }
            Item::Enum(e) => {
                self.t.defined.insert(e.ident.to_string());
            }
            Item::Union(u) => {
                self.t.defined.insert(u.ident.to_string());
            }
            Item::Trait(tr) => {
                self.t.defined.insert(tr.ident.to_string());
            }
            Item::Mod(m) => {
                self.t.mods.insert(m.ident.to_string());
            }
            _ => {}
        }
        visit::visit_item(self, i);
    }

    fn visit_block(&mut self, _b: &'ast Block) {}
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Type heads
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A generic-parameter binding frame for alias expansion. `uses` and `krate` are where the
/// frame's names resolve: the scanned file, or the file and crate that define an expanded alias.
struct Frame<'f> {
    binds: Vec<(String, &'f Type)>,
    uses: &'f UseMap,
    krate: usize,
    parent: Option<&'f Frame<'f>>,
}

struct TypeCx<'a> {
    vocab: &'a Vocab,
    tables: &'a Tables,
    files: &'a [SrcFile],
    /// Every scanned crate's aliases (review W5), and the index of the crate being visited.
    crates: &'a [CrateAliases],
    me: usize,
}

impl TypeCx<'_> {
    /// The canonical name of a single-identifier path after `use` renames.
    fn canon(uses: &UseMap, ident: &str) -> String {
        uses.get(ident)
            .and_then(|p| p.last().cloned())
            .unwrap_or_else(|| ident.to_owned())
    }

    fn box_container(seg: &syn::PathSegment) -> String {
        if let PathArguments::AngleBracketed(ab) = &seg.arguments
            && let Some(GenericArgument::Type(t)) = ab.args.first()
        {
            return match t {
                Type::TraitObject(_) => "Box<dyn>".to_owned(),
                Type::Slice(_) => "Box<[T]>".to_owned(),
                Type::Path(p) if p.path.is_ident("str") => "Box<str>".to_owned(),
                _ => "Box<T>".to_owned(),
            };
        }
        "Box<T>".to_owned()
    }

    /// The scanned crate whose alias table a type path resolves in, or `None` for a path into
    /// std or a crate that is not scanned (review W5). A bare name resolves in the frame's crate
    /// unless a `use` imports it from elsewhere; a qualified or imported path resolves by its
    /// leading segment: `crate` / `self` / `super` or a module of the frame's crate, or the lib
    /// name of a scanned crate. Aliases are keyed by name per crate, so two same-named aliases
    /// with different heads stay ALIAS-AMBIGUOUS, qualified or not.
    fn alias_crate(&self, path: &syn::Path, frame: &Frame<'_>) -> Option<usize> {
        let first = path.segments.first()?.ident.to_string();
        let by_lib = |name: &str| self.crates.iter().position(|c| c.lib == name);
        if path.leading_colon.is_some() {
            return by_lib(&first);
        }
        let lead = match frame.uses.get(&first) {
            Some(full) => full.first().cloned().unwrap_or_else(|| first.clone()),
            None if path.segments.len() == 1 => return Some(frame.krate),
            None => first,
        };
        match lead.as_str() {
            "crate" | "self" | "super" => Some(frame.krate),
            _ if self.crates[frame.krate].mods.contains(&lead) => Some(frame.krate),
            _ => by_lib(&lead),
        }
    }

    /// The first heap head in pre-order, or `None`. `mut_ref` lets the walk descend through
    /// top-level `&mut` (the param rule).
    fn head(
        &self,
        ty: &Type,
        frame: &Frame<'_>,
        mut_ref: bool,
        depth: u32,
        fail: &mut Vec<String>,
        at: &str,
    ) -> Option<String> {
        if depth > 32 {
            fail.push(format!("ALIAS-CYCLE  {at}  type expansion deeper than 32"));
            return None;
        }
        match ty {
            Type::Path(tp) => {
                if tp.qself.is_some() {
                    return None;
                }
                let segs = &tp.path.segments;
                let last = segs.last()?;
                let ident = last.ident.to_string();
                if segs.len() == 1 && tp.path.leading_colon.is_none() {
                    // A generic parameter bound by an enclosing alias expansion.
                    let mut f = Some(frame);
                    while let Some(fr) = f {
                        if let Some((_, t)) = fr.binds.iter().find(|(n, _)| *n == ident) {
                            let parent = fr.parent.unwrap_or(fr);
                            return self.head(t, parent, false, depth + 1, fail, at);
                        }
                        f = fr.parent;
                    }
                }
                let name = if segs.len() == 1 && tp.path.leading_colon.is_none() {
                    TypeCx::canon(frame.uses, &ident)
                } else {
                    ident
                };
                if name == "PhantomData" {
                    return None;
                }
                if self.vocab.is_head(&name) {
                    return Some(match name.as_str() {
                        "Box" => TypeCx::box_container(last),
                        _ => name,
                    });
                }
                let args: Vec<&Type> = match &last.arguments {
                    PathArguments::AngleBracketed(ab) => ab
                        .args
                        .iter()
                        .filter_map(|a| match a {
                            GenericArgument::Type(t) => Some(t),
                            GenericArgument::AssocType(at) => Some(&at.ty),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                if let Some(k) = self.alias_crate(&tp.path, frame)
                    && let Some(defs) = self.crates[k].aliases.get(&name)
                {
                    let n_args = args.len();
                    let fitting: Vec<&AliasDef> =
                        defs.iter().filter(|d| d.params.len() == n_args).collect();
                    if !fitting.is_empty() {
                        let mut results: BTreeSet<Option<String>> = BTreeSet::new();
                        for d in &fitting {
                            let binds: Vec<(String, &Type)> =
                                d.params.iter().cloned().zip(args.iter().copied()).collect();
                            let inner = Frame {
                                binds,
                                uses: &self.crates[k].uses[d.file_idx],
                                krate: k,
                                parent: Some(frame),
                            };
                            results.insert(self.head(&d.ty, &inner, false, depth + 1, fail, at));
                        }
                        if results.len() > 1 {
                            fail.push(format!(
                                "ALIAS-AMBIGUOUS  {at}  `{name}` has {} definitions with different heads {:?}",
                                fitting.len(),
                                results
                            ));
                            return None;
                        }
                        return results.into_iter().next().flatten();
                    }
                }
                args.into_iter()
                    .find_map(|t| self.head(t, frame, false, depth + 1, fail, at))
            }
            Type::Reference(r) => {
                if mut_ref && r.mutability.is_some() {
                    self.head(&r.elem, frame, true, depth + 1, fail, at)
                } else {
                    None
                }
            }
            Type::Array(a) => self.head(&a.elem, frame, false, depth + 1, fail, at),
            Type::Slice(s) => self.head(&s.elem, frame, false, depth + 1, fail, at),
            Type::Tuple(t) => t
                .elems
                .iter()
                .find_map(|e| self.head(e, frame, false, depth + 1, fail, at)),
            Type::Paren(p) => self.head(&p.elem, frame, mut_ref, depth + 1, fail, at),
            Type::Group(g) => self.head(&g.elem, frame, mut_ref, depth + 1, fail, at),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Constructor vocabulary
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Associated functions of a heap head that construct one.
fn is_ctor_fn(name: &str) -> bool {
    name == "new"
        || name.starts_with("new_")
        || name.starts_with("with_capacity")
        || name.starts_with("try_with_capacity")
        || matches!(
            name,
            "from"
                | "from_iter"
                | "from_utf8_lossy"
                | "from_utf16"
                | "from_utf16_lossy"
                | "from_str"
                | "default"
                | "pin"
        )
}

/// Method constructors: name → (container, required arg count or None for any).
fn method_ctor(name: &str, n_args: usize) -> Option<&'static str> {
    Some(match (name, n_args) {
        ("to_string", 0) => "String",
        ("to_vec", 0) => "Vec",
        ("to_owned", 0) => "inferred",
        ("into_owned", 0) => "inferred",
        ("into_boxed_slice", 0) => "Box<[T]>",
        ("into_boxed_str", 0) => "Box<str>",
        ("to_path_buf", 0) => "PathBuf",
        ("to_os_string", 0) => "OsString",
        ("to_string_lossy", 0) => "Cow",
        ("to_lowercase" | "to_uppercase" | "to_ascii_lowercase" | "to_ascii_uppercase", 0) => {
            "inferred"
        }
        ("repeat", 1) => "inferred",
        ("replace", 2) | ("replacen", 3) => "String",
        ("join", 1) => "inferred",
        ("concat", 0) => "inferred",
        ("split_off", 1) => "inferred",
        // Stable sorts allocate their merge scratch (driftsort's BufT) inside std.
        ("sort", 0) | ("sort_by" | "sort_by_key" | "sort_by_cached_key", 1) => "sort-scratch",
        _ => return None,
    })
}

/// Allocating std free functions, matched on the resolved path with `std::`/`core::`/`alloc::`
/// stripped. The container is the call's own name (the UTF-16 path buffer, the env string, …).
fn std_alloc_call(path: &str) -> Option<String> {
    const ENV: &[&str] = &[
        "var",
        "var_os",
        "vars",
        "vars_os",
        "args",
        "args_os",
        "current_dir",
        "current_exe",
        "temp_dir",
        "home_dir",
        "join_paths",
        "split_paths",
    ];
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    if n >= 2 {
        let (m, f) = (segs[n - 2], segs[n - 1]);
        if m == "env" && ENV.contains(&f) {
            return Some(format!("env::{f}"));
        }
        if m == "fs" && f.chars().next().is_some_and(char::is_lowercase) {
            return Some(format!("fs::{f}"));
        }
        if m == "File" && (f == "open" || f == "create" || f == "create_new") {
            return Some(format!("File::{f}"));
        }
        if m == "thread" && f == "spawn" {
            return Some("thread::spawn".to_owned());
        }
        if m == "Command" && f == "new" {
            return Some("Command".to_owned());
        }
        // `io::Error::new` / `other` box a `Custom` and the payload (a `&str` payload's String
        // too); `from(kind)` and `last_os_error` do not allocate (review W4).
        if n >= 3 && segs[n - 3] == "io" && m == "Error" && (f == "new" || f == "other") {
            return Some("io::Error".to_owned());
        }
    }
    None
}

fn is_raw_alloc(path: &str) -> bool {
    let segs: Vec<&str> = path.split("::").collect();
    let n = segs.len();
    n >= 2 && segs[n - 2] == "alloc" && matches!(segs[n - 1], "alloc" | "alloc_zeroed" | "realloc")
}

/// Method calls that are thread / process spawns when their receiver chain starts at the right
/// constructor.
fn chain_root(e: &Expr) -> &Expr {
    match e {
        Expr::MethodCall(m) => chain_root(&m.receiver),
        Expr::Try(t) => chain_root(&t.expr),
        Expr::Paren(p) => chain_root(&p.expr),
        other => other,
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Token scan (macro bodies, unparsed invocations, quote! templates)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One token-scan hit: (container, span, text).
type TokHit = (String, Span, String);

fn token_scan(ts: TokenStream, vocab: &Vocab, uses: &UseMap, out: &mut Vec<TokHit>) {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    for i in 0..toks.len() {
        match &toks[i] {
            TokenTree::Group(g) => token_scan(g.stream(), vocab, uses, out),
            TokenTree::Ident(id) => {
                let prev_dollar_or_hash = i > 0
                    && matches!(&toks[i - 1], TokenTree::Punct(p) if p.as_char() == '$' || p.as_char() == '#');
                if prev_dollar_or_hash {
                    continue;
                }
                let s = id.to_string();
                let next_bang =
                    matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if next_bang && (s == "vec" || s == "format") {
                    let c = if s == "vec" { "Vec" } else { "String" };
                    out.push((c.to_owned(), id.span(), format!("{s}!")));
                    continue;
                }
                let prev_dot =
                    i > 0 && matches!(&toks[i - 1], TokenTree::Punct(p) if p.as_char() == '.');
                if prev_dot {
                    let n_args = match toks.get(i + 1) {
                        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                            let inner = g.stream();
                            if inner.is_empty() {
                                0
                            } else {
                                1 + inner
                                    .into_iter()
                                    .filter(
                                        |t| matches!(t, TokenTree::Punct(p) if p.as_char() == ','),
                                    )
                                    .count()
                            }
                        }
                        _ => usize::MAX,
                    };
                    if s == "collect" {
                        out.push(("inferred".to_owned(), id.span(), ".collect".to_owned()));
                        continue;
                    }
                    if n_args != usize::MAX
                        && let Some(c) = method_ctor(&s, n_args)
                    {
                        out.push((c.to_owned(), id.span(), format!(".{s}")));
                    }
                    continue;
                }
                let name = TypeCx::canon(uses, &s);
                // `Head::f` counts only when `f` constructs (`Box::leak`, `Box::from_raw`,
                // `Arc::clone` hand back an existing allocation).
                let path_call = matches!(
                    (toks.get(i + 1), toks.get(i + 2), toks.get(i + 3)),
                    (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)), Some(TokenTree::Ident(f)))
                        if a.as_char() == ':' && b.as_char() == ':' && !is_ctor_fn(&f.to_string())
                );
                if vocab.is_head(&name) && !path_call {
                    let c = if name == "Box" {
                        let lt = matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '<');
                        match (lt, toks.get(i + 2)) {
                            (true, Some(TokenTree::Ident(d))) if d == "dyn" => {
                                "Box<dyn>".to_owned()
                            }
                            (true, Some(TokenTree::Group(g)))
                                if g.delimiter() == Delimiter::Bracket =>
                            {
                                "Box<[T]>".to_owned()
                            }
                            (true, Some(TokenTree::Ident(d))) if d == "str" => {
                                "Box<str>".to_owned()
                            }
                            _ => "Box<T>".to_owned(),
                        }
                    } else {
                        name
                    };
                    out.push((c, id.span(), s));
                }
            }
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// thread_local! bodies
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct TlsDecl {
    attrs: Vec<Attribute>,
    ident: syn::Ident,
    ty: Type,
    init: Expr,
}

struct TlsBody(Vec<TlsDecl>);

impl Parse for TlsBody {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut v = Vec::new();
        while !input.is_empty() {
            let attrs = input.call(Attribute::parse_outer)?;
            let _vis: syn::Visibility = input.parse()?;
            let _: Token![static] = input.parse()?;
            let ident: syn::Ident = input.parse()?;
            let _: Token![:] = input.parse()?;
            let ty: Type = input.parse()?;
            let _: Token![=] = input.parse()?;
            let init: Expr = input.parse()?;
            if input.peek(Token![;]) {
                let _: Token![;] = input.parse()?;
            }
            v.push(TlsDecl {
                attrs,
                ident,
                ty,
                init,
            });
        }
        Ok(TlsBody(v))
    }
}

/// `vec![x; n]`.
struct RepeatBody(Expr, Expr);

impl Parse for RepeatBody {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let a: Expr = input.parse()?;
        let _: Token![;] = input.parse()?;
        let b: Expr = input.parse()?;
        Ok(RepeatBody(a, b))
    }
}

/// std macros whose bodies parse as comma-separated expressions and are walked as such.
const STD_EXPR_MACROS: &[&str] = &[
    "vec",
    "format",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "panic",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "unreachable",
    "todo",
    "unimplemented",
    "format_args",
    "dbg",
];

/// Macros whose bodies carry no runtime code: nothing to scan.
const INERT_MACROS: &[&str] = &[
    "concat",
    "stringify",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
    "line",
    "column",
    "file",
    "module_path",
    "cfg",
    "compile_error",
    "matches",
];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The visitor
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct Visitor<'a> {
    crate_dir: &'a str,
    file: &'a str,
    mode: Mode,
    physics: bool,
    tcx: &'a TypeCx<'a>,
    uses: &'a UseMap,
    line_of: LineOf<'a>,
    stack: Vec<String>,
    /// Self types of the enclosing impls (for `self.field.clone()`).
    self_ty: Vec<Option<Type>>,
    /// Scopes of locals and params whose type the syntax pins: name → type.
    locals: Vec<BTreeMap<String, Typed>>,
    absorbed: BTreeSet<usize>,
    /// A binding for the next closure's first parameter (`TLS.with(|x| ..)`).
    pending_closure_bind: Option<Typed>,
    stmt_line: Option<u32>,
    sites: &'a mut Vec<Site>,
    slice: &'a mut Vec<SliceHit>,
    includes: &'a mut Vec<(String, String)>,
    fail: &'a mut Vec<String>,
}

fn clip(s: &str) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        flat.chars().take(117).collect::<String>() + "..."
    } else {
        flat
    }
}

fn type_label(ty: &Type) -> String {
    match ty {
        Type::Path(p) if p.qself.is_none() => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        Type::Reference(r) => format!("&{}", type_label(&r.elem)),
        Type::Slice(s) => format!("[{}]", type_label(&s.elem)),
        Type::Paren(p) => type_label(&p.elem),
        Type::Group(g) => type_label(&g.elem),
        other => other.to_token_stream().to_string().replace(' ', ""),
    }
}

fn block_tail(b: &Block) -> Option<&Expr> {
    match b.stmts.last() {
        Some(Stmt::Expr(e, None)) => Some(e),
        _ => None,
    }
}

impl<'a> Visitor<'a> {
    fn at(&self) -> String {
        format!("{}::{}", self.file, self.stack.join("::"))
    }

    fn path_of(&self, leaf: &str) -> String {
        if self.stack.is_empty() {
            leaf.to_owned()
        } else {
            format!("{}::{}", self.stack.join("::"), leaf)
        }
    }

    fn fn_path(&self) -> String {
        self.stack.join("::")
    }

    fn frame(&self) -> Frame<'a> {
        Frame {
            binds: Vec::new(),
            uses: self.uses,
            krate: self.tcx.me,
            parent: None,
        }
    }

    fn head_of(&mut self, ty: &Type, mut_ref: bool) -> Option<String> {
        let at = self.at();
        let frame = self.frame();
        let mut fail = Vec::new();
        let h = self.tcx.head(ty, &frame, mut_ref, 0, &mut fail, &at);
        self.fail.extend(fail);
        h
    }

    fn push_site(
        &mut self,
        shape: Shape,
        item_path: String,
        container: String,
        span: Span,
        text: String,
    ) {
        if self.mode != Mode::Full && shape != Shape::Emitted {
            return;
        }
        let line = (self.line_of)(span);
        self.sites.push(Site {
            crate_dir: self.crate_dir.to_owned(),
            file: self.file.to_owned(),
            shape,
            item_path,
            container,
            text: clip(&text),
            line,
            stmt_line: self.stmt_line.or(line),
        });
    }

    /// The container a constructor expression produces, if it is one (for absorption).
    fn ctor_container_of(&mut self, e: &Expr) -> Option<String> {
        match e {
            Expr::Call(c) => {
                if let Expr::Path(ep) = &*c.func {
                    let rp = self.resolved_call_path(&ep.path);
                    if let Some(k) = std_alloc_call(&rp) {
                        return Some(k);
                    }
                    if let Some(k) = self.head_ctor_path(ep) {
                        return Some(k);
                    }
                    return self.ufcs_ctor(ep, &c.args);
                }
                None
            }
            Expr::MethodCall(m) => {
                let name = m.method.to_string();
                if name == "collect" {
                    return Some(self.collect_container(m));
                }
                method_ctor(&name, m.args.len()).map(str::to_owned)
            }
            Expr::Macro(m) => {
                if m.mac.path.is_ident("vec") {
                    Some("Vec".to_owned())
                } else if m.mac.path.is_ident("format") {
                    Some("String".to_owned())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn collect_container(&mut self, m: &syn::ExprMethodCall) -> String {
        self.turbofish_head(m.turbofish.as_ref())
            .unwrap_or_else(|| "inferred".to_owned())
    }

    /// The heap head of a turbofish's first type argument (`::<Vec<_>>`), if it names one.
    fn turbofish_head(
        &mut self,
        tf: Option<&syn::AngleBracketedGenericArguments>,
    ) -> Option<String> {
        let t = tf?.args.iter().find_map(|a| match a {
            GenericArgument::Type(t) => Some(t.clone()),
            _ => None,
        })?;
        self.head_of(&t, false)
    }

    /// A method constructor called through its path, with the receiver as the first argument
    /// (`ToString::to_string(&x)`, `str::to_owned(s)`, `<[u8]>::to_vec(v)`, `Vec::clone(&v)`,
    /// `Iterator::collect::<Vec<_>>(it)`): the allocation the method-call form counts (review O1,
    /// tester N1). The qualifier must be a type or a trait (upper-case, `str`, or a `<T>::`
    /// self type), so a module's free function that shares a method's name is not one.
    fn ufcs_ctor(
        &mut self,
        ep: &syn::ExprPath,
        args: &Punctuated<Expr, Token![,]>,
    ) -> Option<String> {
        let segs = &ep.path.segments;
        let last = segs.last()?;
        // The qualifier's own path segment, when it has one (for `Box`'s container shape).
        let qual_seg: Option<&syn::PathSegment> = match &ep.qself {
            Some(q) => match &*q.ty {
                Type::Path(tp) if tp.qself.is_none() => tp.path.segments.last(),
                _ => None,
            },
            None if segs.len() >= 2 => Some(&segs[segs.len() - 2]),
            None => return None,
        };
        let qualifier = match (&ep.qself, qual_seg) {
            (Some(q), _) => Visitor::type_name(&q.ty).unwrap_or_default(),
            (None, Some(s)) if segs.len() == 2 && ep.path.leading_colon.is_none() => {
                TypeCx::canon(self.uses, &s.ident.to_string())
            }
            (None, Some(s)) => s.ident.to_string(),
            (None, None) => return None,
        };
        let is_type = ep.qself.is_some()
            || qualifier == "str"
            || qualifier.chars().next().is_some_and(char::is_uppercase);
        if !is_type {
            return None;
        }
        let name = last.ident.to_string();
        let n = args.len().checked_sub(1)?;
        let turbofish = match &last.arguments {
            PathArguments::AngleBracketed(ab) => Some(ab),
            _ => None,
        };
        match (name.as_str(), n) {
            ("collect", 0) => Some(
                self.turbofish_head(turbofish)
                    .unwrap_or_else(|| "inferred".to_owned()),
            ),
            ("parse", 0) => self.turbofish_head(turbofish),
            ("clone", 0) if self.tcx.vocab.is_head(&qualifier) => match qualifier.as_str() {
                "Arc" | "Rc" => None,
                "Box" => Some(
                    qual_seg
                        .map(TypeCx::box_container)
                        .unwrap_or_else(|| "Box<T>".to_owned()),
                ),
                _ => Some(qualifier),
            },
            ("clone", 0) if qualifier == "Clone" => {
                let recv = match &args[0] {
                    Expr::Reference(r) => &*r.expr,
                    other => other,
                };
                self.clone_receiver_head(recv)
            }
            _ => method_ctor(&name, n).map(str::to_owned),
        }
    }

    /// Marks the constructors that form a declaration's value: the receiver spine, `?`, block
    /// and branch tails, closure bodies and the arguments of non-head wrapper constructors
    /// (`Mutex::new(Vec::new())`, `Some(vec![])`, `LazyLock::new(|| HashMap::new())`). A spine
    /// constructor of a DIFFERENT container is a second allocation and stays a site
    /// (`let a: Arc<[T]> = it.collect::<Vec<_>>().into()` is two).
    fn absorb(&mut self, e: &Expr, decl: &str) {
        let compatible = |c: &str| {
            c == decl
                || c == "inferred"
                || decl == "inferred"
                || (c.starts_with("Box") && decl.starts_with("Box"))
        };
        match e {
            Expr::Try(t) => self.absorb(&t.expr, decl),
            Expr::Paren(p) => self.absorb(&p.expr, decl),
            Expr::Group(g) => self.absorb(&g.expr, decl),
            Expr::MethodCall(m) => {
                if self.ctor_container_of(e).is_some_and(|c| compatible(&c)) {
                    self.absorbed
                        .insert(m as *const syn::ExprMethodCall as usize);
                }
                self.absorb(&m.receiver, decl);
            }
            Expr::Macro(m) => {
                if self.ctor_container_of(e).is_some_and(|c| compatible(&c)) {
                    self.absorbed.insert(&m.mac as *const Macro as usize);
                }
            }
            Expr::Block(b) => {
                if let Some(t) = block_tail(&b.block) {
                    self.absorb(t, decl);
                }
            }
            Expr::Unsafe(b) => {
                if let Some(t) = block_tail(&b.block) {
                    self.absorb(t, decl);
                }
            }
            Expr::Const(b) => {
                if let Some(t) = block_tail(&b.block) {
                    self.absorb(t, decl);
                }
            }
            Expr::If(i) => {
                if let Some(t) = block_tail(&i.then_branch) {
                    self.absorb(t, decl);
                }
                if let Some((_, e)) = &i.else_branch {
                    self.absorb(e, decl);
                }
            }
            Expr::Match(m) => {
                for a in &m.arms {
                    self.absorb(&a.body, decl);
                }
            }
            Expr::Closure(c) => self.absorb(&c.body, decl),
            Expr::Call(c) => {
                let cc = self.ctor_container_of(e);
                if cc.as_deref().is_some_and(compatible) {
                    self.absorbed.insert(c as *const syn::ExprCall as usize);
                }
                if cc.is_none() {
                    for a in &c.args {
                        self.absorb(a, decl);
                    }
                }
            }
            _ => {}
        }
    }

    fn is_absorbed<T>(&self, node: &T) -> bool {
        self.absorbed.contains(&(node as *const T as usize))
    }

    /// `(container)` if `func` is a path to a heap head's constructor.
    fn head_ctor(&mut self, func: &Expr) -> Option<String> {
        let Expr::Path(ep) = func else { return None };
        self.head_ctor_path(ep)
    }

    fn head_ctor_path(&mut self, ep: &syn::ExprPath) -> Option<String> {
        let segs = &ep.path.segments;
        let fname = segs.last()?.ident.to_string();
        if !is_ctor_fn(&fname) {
            return None;
        }
        let (tyname, seg) = if let Some(q) = &ep.qself {
            match &*q.ty {
                Type::Path(tp) if q.position == 0 => {
                    let s = tp.path.segments.last()?;
                    (s.ident.to_string(), Some(s.clone()))
                }
                _ => return None,
            }
        } else {
            if segs.len() < 2 {
                return None;
            }
            let s = &segs[segs.len() - 2];
            let name = if segs.len() == 2 && ep.path.leading_colon.is_none() {
                TypeCx::canon(self.uses, &s.ident.to_string())
            } else {
                s.ident.to_string()
            };
            (name, Some(s.clone()))
        };
        if !self.tcx.vocab.is_head(&tyname) {
            return self.alias_ctor(ep, &tyname);
        }
        Some(match tyname.as_str() {
            "Box" => {
                let from_seg = seg
                    .map(|s| TypeCx::box_container(&s))
                    .unwrap_or_else(|| "Box<T>".to_owned());
                if from_seg == "Box<T>" && fname.contains("slice") {
                    "Box<[T]>".to_owned()
                } else {
                    from_seg
                }
            }
            _ => tyname,
        })
    }

    /// A constructor called through a heap alias (`Contour::new()` for
    /// `type Contour = Vec<Segment>`, `m::Buf::with_capacity(n)`): the aliased head's constructor
    /// (review W5, the alias hole's constructor form). Only a qualifier that resolves to an alias
    /// counts, so `Option::<Vec<u8>>::default()` stays no site.
    fn alias_ctor(&mut self, ep: &syn::ExprPath, tyname: &str) -> Option<String> {
        let segs = &ep.path.segments;
        if ep.qself.is_some() || segs.len() < 2 {
            return None;
        }
        let qualifier = syn::Path {
            leading_colon: ep.path.leading_colon,
            segments: segs.iter().take(segs.len() - 1).cloned().collect(),
        };
        let k = self.tcx.alias_crate(&qualifier, &self.frame())?;
        self.tcx.crates[k].aliases.get(tyname)?;
        let ty = Type::Path(syn::TypePath {
            qself: None,
            path: qualifier,
        });
        self.head_of(&ty, false)
    }

    /// A constructor named as a VALUE (`.map(PathBuf::from)`, `.map(Vec::into_boxed_slice)`,
    /// `.map(ToString::to_string)`): the call happens inside the adapter, the site is here.
    fn ctor_as_value(&mut self, ep: &syn::ExprPath) -> Option<String> {
        if ep.path.segments.len() < 2 && ep.qself.is_none() {
            return None;
        }
        if let Some(c) = self.head_ctor_path(ep) {
            return Some(c);
        }
        let last = ep.path.segments.last()?.ident.to_string();
        match last.as_str() {
            "to_string" | "to_owned" | "to_vec" | "into_boxed_slice" | "into_boxed_str"
            | "to_path_buf" | "into_owned" => method_ctor(&last, 0).map(str::to_owned),
            _ => None,
        }
    }

    /// The path of a call, with a leading `use`-imported segment expanded, `std::`/`core::`/
    /// `alloc::` and `::` stripped.
    fn resolved_call_path(&self, p: &syn::Path) -> String {
        let mut segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        if p.leading_colon.is_none()
            && let Some(first) = segs.first()
            && let Some(full) = self.uses.get(first)
        {
            let mut v = full.clone();
            v.extend(segs.drain(1..));
            segs = v;
        }
        while segs
            .first()
            .is_some_and(|s| s == "std" || s == "core" || s == "crate")
        {
            segs.remove(0);
        }
        if segs.first().is_some_and(|s| s == "alloc") && segs.len() > 2 {
            segs.remove(0);
        }
        segs.join("::")
    }

    fn local_type(&self, name: &str) -> Option<Typed> {
        self.locals.iter().rev().find_map(|s| s.get(name).cloned())
    }

    fn bind_local(&mut self, name: String, t: Typed) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, t);
        }
    }

    fn type_name(ty: &Type) -> Option<String> {
        match ty {
            Type::Reference(r) => Visitor::type_name(&r.elem),
            Type::Path(p) if p.qself.is_none() => {
                p.path.segments.last().map(|s| s.ident.to_string())
            }
            Type::Paren(p) => Visitor::type_name(&p.elem),
            _ => None,
        }
    }

    /// A single-segment path type named `name` (for types synthesised from constructors).
    fn named_type(name: &str) -> Type {
        syn::parse_str::<Type>(name).unwrap_or_else(|_| Type::Verbatim(TokenStream::new()))
    }

    /// Strips references and ONE transparent wrapper layer (`RefCell<T>`, `Mutex<T>`,
    /// `Option<T>`, a guard, …) — what `.borrow()`, `.lock()`, `.unwrap()` hand back.
    fn peel(ty: &Type) -> Type {
        let t = match ty {
            Type::Reference(r) => (*r.elem).clone(),
            other => other.clone(),
        };
        if let Type::Path(p) = &t
            && let Some(seg) = p.path.segments.last()
            && matches!(
                seg.ident.to_string().as_str(),
                "RefCell"
                    | "Mutex"
                    | "RwLock"
                    | "Option"
                    | "Result"
                    | "Cell"
                    | "OnceLock"
                    | "OnceCell"
                    | "LazyLock"
                    | "Ref"
                    | "RefMut"
                    | "MutexGuard"
                    | "RwLockReadGuard"
                    | "RwLockWriteGuard"
                    | "ManuallyDrop"
                    | "UnsafeCell"
            )
            && let PathArguments::AngleBracketed(ab) = &seg.arguments
            && let Some(GenericArgument::Type(inner)) = ab.args.first()
        {
            return inner.clone();
        }
        t
    }

    fn field_type(&self, base: &Typed, member: &str) -> Option<Typed> {
        let sname = Visitor::type_name(&Visitor::peel(&base.ty))?;
        let fields = self.tcx.tables.structs.get(&sname)?;
        let (_, ty, idx) = fields.iter().find(|(n, _, _)| n == member)?;
        Some(Typed {
            ty: ty.clone(),
            file_idx: Some(*idx),
        })
    }

    fn unique(v: Option<&Vec<Typed>>) -> Option<Typed> {
        let v = v?;
        let first = v.first()?;
        let key = first.ty.to_token_stream().to_string();
        v.iter()
            .all(|t| t.ty.to_token_stream().to_string() == key)
            .then(|| first.clone())
    }

    /// The type of an expression, where the syntax pins it: typed locals and params, untyped
    /// locals bound to a constructor or a crate fn's result, `self` and field chains, the
    /// transparent accessors (`borrow`, `lock`, `unwrap`, …), crate fns and methods by their
    /// declared return type, and indexing into a sequence. `None` otherwise — never a guess.
    fn infer(&mut self, e: &Expr, depth: u32) -> Option<Typed> {
        if depth > 12 {
            return None;
        }
        match e {
            Expr::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
                let n = p.path.segments[0].ident.to_string();
                if n == "self" {
                    let t = self.self_ty.last().cloned().flatten()?;
                    return Some(Typed {
                        ty: t,
                        file_idx: None,
                    });
                }
                self.local_type(&n)
            }
            Expr::Field(f) => {
                let syn::Member::Named(member) = &f.member else {
                    return None;
                };
                let base = self.infer(&f.base, depth + 1)?;
                self.field_type(&base, &member.to_string())
            }
            Expr::Paren(p) => self.infer(&p.expr, depth + 1),
            Expr::Group(g) => self.infer(&g.expr, depth + 1),
            Expr::Reference(r) => self.infer(&r.expr, depth + 1),
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => {
                let t = self.infer(&u.expr, depth + 1)?;
                Some(Typed {
                    ty: Visitor::peel(&t.ty),
                    file_idx: t.file_idx,
                })
            }
            Expr::Index(ix) => {
                let t = self.infer(&ix.expr, depth + 1)?;
                let inner = Visitor::peel(&t.ty);
                match &inner {
                    Type::Slice(s) => Some(Typed {
                        ty: (*s.elem).clone(),
                        file_idx: t.file_idx,
                    }),
                    Type::Array(a) => Some(Typed {
                        ty: (*a.elem).clone(),
                        file_idx: t.file_idx,
                    }),
                    Type::Path(p) => {
                        let seg = p.path.segments.last()?;
                        let PathArguments::AngleBracketed(ab) = &seg.arguments else {
                            return None;
                        };
                        let tys: Vec<&Type> = ab
                            .args
                            .iter()
                            .filter_map(|a| match a {
                                GenericArgument::Type(t) => Some(t),
                                _ => None,
                            })
                            .collect();
                        let pick = match seg.ident.to_string().as_str() {
                            "Vec" | "VecDeque" | "Box" => tys.first(),
                            "HashMap" | "BTreeMap" => tys.get(1),
                            _ => None,
                        }?;
                        Some(Typed {
                            ty: (*pick).clone(),
                            file_idx: t.file_idx,
                        })
                    }
                    _ => None,
                }
            }
            Expr::MethodCall(m) => {
                let name = m.method.to_string();
                match name.as_str() {
                    "borrow" | "borrow_mut" | "lock" | "read" | "write" | "get" | "get_mut"
                    | "as_ref" | "as_mut" | "unwrap" | "expect" | "as_deref" | "as_deref_mut"
                    | "deref" | "deref_mut" | "unwrap_or_default" | "get_or_init" => {
                        let t = self.infer(&m.receiver, depth + 1)?;
                        Some(Typed {
                            ty: Visitor::peel(&t.ty),
                            file_idx: t.file_idx,
                        })
                    }
                    "clone" | "to_owned" => self.infer(&m.receiver, depth + 1),
                    "collect" => {
                        let tf = m.turbofish.as_ref()?;
                        tf.args.iter().find_map(|a| match a {
                            GenericArgument::Type(t) => Some(Typed {
                                ty: t.clone(),
                                file_idx: None,
                            }),
                            _ => None,
                        })
                    }
                    _ => {
                        if let Some(c) = method_ctor(&name, m.args.len())
                            && c != "inferred"
                            && c != "sort-scratch"
                        {
                            let base = c.split('<').next().unwrap_or(c);
                            return Some(Typed {
                                ty: Visitor::named_type(base),
                                file_idx: None,
                            });
                        }
                        let recv = self.infer(&m.receiver, depth + 1)?;
                        let tname = Visitor::type_name(&Visitor::peel(&recv.ty))?;
                        let r = Visitor::unique(self.tcx.tables.methods.get(&(tname, name)))?;
                        Some(r)
                    }
                }
            }
            Expr::Call(c) => {
                let Expr::Path(ep) = &*c.func else {
                    return None;
                };
                if let Some(h) = self.head_ctor_path(ep) {
                    let base = h.split('<').next().unwrap_or(&h).to_owned();
                    return Some(Typed {
                        ty: Visitor::named_type(&base),
                        file_idx: None,
                    });
                }
                let segs = &ep.path.segments;
                if segs.len() == 1 {
                    return Visitor::unique(self.tcx.tables.fns.get(&segs[0].ident.to_string()));
                }
                let tname = if segs.len() == 2 && ep.path.leading_colon.is_none() {
                    TypeCx::canon(self.uses, &segs[0].ident.to_string())
                } else {
                    segs[segs.len() - 2].ident.to_string()
                };
                let fname = segs[segs.len() - 1].ident.to_string();
                if let Some(r) =
                    Visitor::unique(self.tcx.tables.methods.get(&(tname.clone(), fname.clone())))
                {
                    return Some(r);
                }
                // `X::new(..)` of a type with no table entry: the value is an `X`.
                (fname == "new" || fname == "default" || fname.starts_with("with_")).then(|| {
                    Typed {
                        ty: Visitor::named_type(&tname),
                        file_idx: None,
                    }
                })
            }
            Expr::Macro(m) => {
                if m.mac.path.is_ident("vec") {
                    Some(Typed {
                        ty: Visitor::named_type("Vec"),
                        file_idx: None,
                    })
                } else if m.mac.path.is_ident("format") {
                    Some(Typed {
                        ty: Visitor::named_type("String"),
                        file_idx: None,
                    })
                } else {
                    None
                }
            }
            Expr::Struct(s) => {
                let seg = s.path.segments.last()?;
                if seg.ident == "Self" {
                    let t = self.self_ty.last().cloned().flatten()?;
                    return Some(Typed {
                        ty: t,
                        file_idx: None,
                    });
                }
                Some(Typed {
                    ty: Visitor::named_type(&seg.ident.to_string()),
                    file_idx: None,
                })
            }
            Expr::Try(t) => {
                let inner = self.infer(&t.expr, depth + 1)?;
                Some(Typed {
                    ty: Visitor::peel(&inner.ty),
                    file_idx: inner.file_idx,
                })
            }
            _ => None,
        }
    }

    fn head_of_typed(&mut self, t: &Typed, mut_ref: bool) -> Option<String> {
        let uses = match t.file_idx {
            Some(i) => &self.tcx.files[i].uses,
            None => self.uses,
        };
        let at = self.at();
        let frame = Frame {
            binds: Vec::new(),
            uses,
            krate: self.tcx.me,
            parent: None,
        };
        let mut fail = Vec::new();
        let h = self.tcx.head(&t.ty, &frame, mut_ref, 0, &mut fail, &at);
        self.fail.extend(fail);
        h
    }

    /// The head a `.clone()` allocates, when the receiver's type is pinned by the syntax
    /// (critique W6). `Arc`/`Rc` clones are refcount increments, not allocations.
    fn clone_receiver_head(&mut self, recv: &Expr) -> Option<String> {
        let t = self.infer(recv, 0)?;
        let peeled = Typed {
            ty: match &t.ty {
                Type::Reference(r) => (*r.elem).clone(),
                other => other.clone(),
            },
            file_idx: t.file_idx,
        };
        match self.head_of_typed(&peeled, false).as_deref() {
            Some("Arc" | "Rc") | None => None,
            Some(h) => Some(h.to_owned()),
        }
    }

    fn fields(&mut self, owner: &str, fields: &Fields, variant: Option<&str>) {
        for (i, f) in fields.iter().enumerate() {
            if excluded(&f.attrs) {
                continue;
            }
            // `&mut Vec<T>` in a field grows the referent like a `&mut` param does.
            let Some(c) = self.head_of(&f.ty, true) else {
                continue;
            };
            let fname = f
                .ident
                .as_ref()
                .map(|i| i.to_string())
                .unwrap_or_else(|| i.to_string());
            let leaf = match variant {
                Some(v) => format!("{owner}::{v}.{fname}"),
                None => format!("{owner}::{fname}"),
            };
            let span = f
                .ident
                .as_ref()
                .map(|i| i.span())
                .unwrap_or_else(|| f.ty.span());
            let text = format!("{fname}: {}", f.ty.to_token_stream());
            let path = self.path_of(&leaf);
            self.push_site(Shape::Field, path, c, span, text);
        }
    }

    fn signature(&mut self, sig: &Signature) {
        // A `-> &mut Vec<T>` hands the caller a growable heap, like a `&mut` param.
        if let ReturnType::Type(_, ty) = &sig.output
            && let Some(c) = self.head_of(ty, true)
        {
            let path = self.fn_path();
            let text = format!("fn {} -> {}", sig.ident, ty.to_token_stream());
            self.push_site(Shape::Ret, path, c, ty.span(), text);
        }
        let mut scope = BTreeMap::new();
        for a in &sig.inputs {
            let FnArg::Typed(pt) = a else { continue };
            if let Pat::Ident(pi) = &*pt.pat {
                scope.insert(
                    pi.ident.to_string(),
                    Typed {
                        ty: (*pt.ty).clone(),
                        file_idx: None,
                    },
                );
            }
            if let Some(c) = self.head_of(&pt.ty, true) {
                let path = self.fn_path();
                let text = pt.to_token_stream().to_string();
                self.push_site(Shape::Param, path, c, pt.ty.span(), text);
            }
        }
        self.locals.push(scope);
    }

    fn fn_like(&mut self, attrs: &[Attribute], sig: &Signature, block: Option<&Block>) {
        if excluded(attrs) {
            return;
        }
        let Some(block) = block else { return };
        self.stack.push(sig.ident.to_string());
        self.signature(sig);
        self.visit_block(block);
        self.locals.pop();
        self.stack.pop();
    }

    /// Absorption is keyed by node address. A macro body is parsed into a TEMPORARY tree that is
    /// freed when the invocation is done, and a later temporary can reuse its addresses; so every
    /// address recorded while a body is walked is forgotten when the walk returns.
    fn macro_invocation(&mut self, mac: &Macro, attrs: &[Attribute]) {
        let saved = self.absorbed.clone();
        self.macro_invocation_inner(mac, attrs);
        self.absorbed = saved;
    }

    fn macro_invocation_inner(&mut self, mac: &Macro, attrs: &[Attribute]) {
        if excluded(attrs) {
            return;
        }
        let name = mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if name == "include" {
            self.includes
                .push((self.file.to_owned(), clip(&mac.tokens.to_string())));
            return;
        }
        if name == "thread_local" {
            match mac.parse_body::<TlsBody>() {
                Ok(body) => {
                    for d in &body.0 {
                        if excluded(&d.attrs) {
                            continue;
                        }
                        let c = self
                            .head_of(&d.ty, false)
                            .unwrap_or_else(|| "tls-key".to_owned());
                        let path = self.path_of(&format!("tls {}", d.ident));
                        let text = format!("static {}: {}", d.ident, d.ty.to_token_stream());
                        self.push_site(Shape::Tls, path, c.clone(), d.ident.span(), text);
                        self.absorb(&d.init, &c);
                        self.stack.push(format!("tls {}", d.ident));
                        self.visit_expr(&d.init);
                        self.stack.pop();
                    }
                }
                Err(e) => self
                    .fail
                    .push(format!("PARSE  {}  thread_local! body: {e}", self.at())),
            }
            return;
        }
        if name == "quote"
            || name == "quote_spanned"
            || name == "parse_quote"
            || name == "parse_quote_spanned"
        {
            if self.mode == Mode::Emitted {
                let mut hits = Vec::new();
                token_scan(mac.tokens.clone(), self.tcx.vocab, self.uses, &mut hits);
                for (c, span, text) in hits {
                    let file = self.file.to_owned();
                    self.push_site(Shape::Emitted, file, c, span, text);
                }
                return;
            }
        } else if self.mode == Mode::Emitted {
            return;
        }
        if INERT_MACROS.contains(&name.as_str()) {
            return;
        }
        if STD_EXPR_MACROS.contains(&name.as_str()) {
            let is_vec = name == "vec";
            let is_format = name == "format";
            if is_vec || is_format {
                let c = if is_vec { "Vec" } else { "String" };
                if !self.is_absorbed(mac) {
                    let path = self.fn_path();
                    let text = mac.to_token_stream().to_string();
                    self.push_site(Shape::Ctor, path, c.to_owned(), mac.path.span(), text);
                }
            }
            if is_vec && let Ok(RepeatBody(a, b)) = mac.parse_body::<RepeatBody>() {
                self.visit_expr(&a);
                self.visit_expr(&b);
                return;
            }
            match mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
                Ok(args) => {
                    for a in &args {
                        self.visit_expr(a);
                    }
                    return;
                }
                Err(_) if is_vec || is_format => return,
                Err(_) => {}
            }
        }
        let mut hits = Vec::new();
        token_scan(mac.tokens.clone(), self.tcx.vocab, self.uses, &mut hits);
        for (c, span, text) in hits {
            self.push_site(Shape::Macro, format!("{name}!"), c, span, text);
        }
    }
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    fn visit_item(&mut self, i: &'ast Item) {
        if excluded(item_attrs(i)) {
            return;
        }
        if self.mode == Mode::Emitted {
            // Only quote! templates are scanned; still descend to find them.
            visit::visit_item(self, i);
            return;
        }
        visit::visit_item(self, i);
    }

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if let Some((_, items)) = &m.content {
            self.stack.push(m.ident.to_string());
            for it in items {
                self.visit_item(it);
            }
            self.stack.pop();
        }
    }

    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        let owner = s.ident.to_string();
        self.fields(&owner, &s.fields, None);
    }

    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        let owner = e.ident.to_string();
        for v in &e.variants {
            if excluded(&v.attrs) {
                continue;
            }
            let vn = v.ident.to_string();
            self.fields(&owner, &v.fields, Some(&vn));
        }
    }

    fn visit_item_union(&mut self, u: &'ast syn::ItemUnion) {
        let owner = u.ident.to_string();
        self.fields(&owner, &Fields::Named(u.fields.clone()), None);
    }

    fn visit_item_type(&mut self, _t: &'ast syn::ItemType) {}

    fn visit_item_const(&mut self, _c: &'ast syn::ItemConst) {}

    fn visit_item_use(&mut self, _u: &'ast syn::ItemUse) {}

    fn visit_item_foreign_mod(&mut self, _f: &'ast syn::ItemForeignMod) {}

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        let name = format!("static {}", s.ident);
        let global_alloc = s
            .attrs
            .iter()
            .any(|a| a.path().is_ident("global_allocator"));
        let c = if global_alloc {
            Some("global_allocator".to_owned())
        } else {
            self.head_of(&s.ty, false)
        };
        if let Some(c) = c {
            let path = self.path_of(&name);
            let text = format!("static {}: {}", s.ident, s.ty.to_token_stream());
            self.push_site(Shape::Static, path, c.clone(), s.ident.span(), text);
            self.absorb(&s.expr, &c);
        }
        self.stack.push(name);
        self.visit_expr(&s.expr);
        self.stack.pop();
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let self_name = type_label(&i.self_ty);
        let label = match &i.trait_ {
            Some((_, p, _)) => {
                let t = p
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default();
                format!("<{self_name} as {t}>")
            }
            None => self_name.clone(),
        };
        self.stack.push(label);
        self.self_ty.push(Some((*i.self_ty).clone()));
        for it in &i.items {
            match it {
                ImplItem::Fn(f) => self.fn_like(&f.attrs, &f.sig, Some(&f.block)),
                ImplItem::Macro(m) => self.macro_invocation(&m.mac, &m.attrs),
                _ => {}
            }
        }
        self.self_ty.pop();
        self.stack.pop();
    }

    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        self.stack.push(t.ident.to_string());
        self.self_ty.push(None);
        for it in &t.items {
            match it {
                TraitItem::Fn(f) => self.fn_like(&f.attrs, &f.sig, f.default.as_ref()),
                TraitItem::Macro(m) => self.macro_invocation(&m.mac, &m.attrs),
                _ => {}
            }
        }
        self.self_ty.pop();
        self.stack.pop();
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.fn_like(&f.attrs, &f.sig, Some(&f.block));
    }

    fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
        if excluded(&m.attrs) {
            return;
        }
        if m.mac.path.is_ident("macro_rules") {
            if self.mode != Mode::Full {
                return;
            }
            let name = m.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
            let mut hits = Vec::new();
            token_scan(m.mac.tokens.clone(), self.tcx.vocab, self.uses, &mut hits);
            for (c, span, text) in hits {
                self.push_site(Shape::Macro, format!("macro_rules!{name}"), c, span, text);
            }
            return;
        }
        self.macro_invocation(&m.mac, &m.attrs);
    }

    fn visit_block(&mut self, b: &'ast Block) {
        self.locals.push(BTreeMap::new());
        for s in &b.stmts {
            self.visit_stmt(s);
        }
        self.locals.pop();
    }

    fn visit_stmt(&mut self, s: &'ast Stmt) {
        let attrs: &[Attribute] = match s {
            Stmt::Local(l) => &l.attrs,
            Stmt::Item(i) => item_attrs(i),
            Stmt::Expr(e, _) => expr_attrs(e),
            Stmt::Macro(m) => &m.attrs,
        };
        if excluded(attrs) {
            return;
        }
        let saved = self.stmt_line;
        self.stmt_line = (self.line_of)(s.span());
        match s {
            Stmt::Macro(m) => self.macro_invocation(&m.mac, &m.attrs),
            other => visit::visit_stmt(self, other),
        }
        self.stmt_line = saved;
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        if excluded(&l.attrs) {
            return;
        }
        let mut bind: Option<(String, Typed)> = None;
        if let Pat::Type(pt) = &l.pat {
            if let Pat::Ident(pi) = &*pt.pat {
                bind = Some((
                    pi.ident.to_string(),
                    Typed {
                        ty: (*pt.ty).clone(),
                        file_idx: None,
                    },
                ));
            }
            if let Some(c) = self.head_of(&pt.ty, false) {
                let path = self.fn_path();
                let text = l.to_token_stream().to_string();
                self.push_site(Shape::Let, path, c.clone(), l.let_token.span, text);
                if let Some(init) = &l.init {
                    self.absorb(&init.expr, &c);
                }
            }
        } else if let Pat::Ident(pi) = &l.pat
            && let Some(init) = &l.init
        {
            if let Some(t) = self.infer(&init.expr, 0) {
                bind = Some((pi.ident.to_string(), t));
            }
            // The builder pattern: `let mut b = thread::Builder::new().name(..); b.spawn(..)`.
            if let Some(rt) = self.chain_root_type(&init.expr) {
                let t = Typed {
                    ty: Visitor::named_type(&rt),
                    file_idx: None,
                };
                self.bind_local(format!("root:{}", pi.ident), t);
            }
        }
        if let Some(init) = &l.init {
            self.visit_expr(&init.expr);
            if let Some((_, e)) = &init.diverge {
                self.visit_expr(e);
            }
        }
        if let Some((n, t)) = bind {
            self.bind_local(n, t);
        }
    }

    fn visit_arm(&mut self, a: &'ast syn::Arm) {
        if excluded(&a.attrs) {
            return;
        }
        visit::visit_arm(self, a);
    }

    fn visit_field_value(&mut self, f: &'ast syn::FieldValue) {
        if excluded(&f.attrs) {
            return;
        }
        visit::visit_field_value(self, f);
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        let mut scope = BTreeMap::new();
        if let Some(t) = self.pending_closure_bind.take()
            && let Some(Pat::Ident(pi)) = c.inputs.first()
        {
            // `TLS.with(|x| ..)`: `x` is `&` the thread-local's declared type.
            scope.insert(pi.ident.to_string(), t);
        }
        for p in &c.inputs {
            if let Pat::Type(pt) = p {
                if let Pat::Ident(pi) = &*pt.pat {
                    scope.insert(
                        pi.ident.to_string(),
                        Typed {
                            ty: (*pt.ty).clone(),
                            file_idx: None,
                        },
                    );
                }
                if let Some(h) = self.head_of(&pt.ty, true) {
                    let path = self.fn_path();
                    let text = pt.to_token_stream().to_string();
                    self.push_site(Shape::Param, path, h, pt.ty.span(), text);
                }
            }
        }
        if let ReturnType::Type(_, ty) = &c.output
            && let Some(h) = self.head_of(ty, false)
        {
            let path = self.fn_path();
            let text = format!("|..| -> {}", ty.to_token_stream());
            self.push_site(Shape::Ret, path, h, ty.span(), text);
        }
        self.locals.push(scope);
        self.visit_expr(&c.body);
        self.locals.pop();
    }

    fn visit_expr(&mut self, e: &'ast Expr) {
        if excluded(expr_attrs(e)) {
            return;
        }
        if let Expr::Macro(m) = e {
            self.macro_invocation(&m.mac, &m.attrs);
            return;
        }
        if let Expr::Path(ep) = e {
            // Only reached for a path used as a VALUE: `visit_expr_call` does not visit its
            // callee path.
            if let Some(c) = self.ctor_as_value(ep) {
                let path = self.fn_path();
                let text = ep.to_token_stream().to_string();
                self.push_site(Shape::Ctor, path, c, ep.span(), text);
            }
            return;
        }
        visit::visit_expr(self, e);
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        let absorbed = self.is_absorbed(c);
        if let Expr::Path(ep) = &*c.func {
            let rp = self.resolved_call_path(&ep.path);
            let last = ep
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if self.physics {
                let canon_last = if ep.path.segments.len() == 1 {
                    TypeCx::canon(self.uses, &last)
                } else {
                    last.clone()
                };
                if canon_last == "try_with_active_pool" {
                    self.slice_hit(
                        "try_with_active_pool",
                        c.to_token_stream().to_string(),
                        c.func.span(),
                    );
                } else if canon_last == "scope" && !rp.split("::").any(|s| s == "thread") {
                    self.slice_hit("scope", c.to_token_stream().to_string(), c.func.span());
                }
            }
            if !absorbed {
                let text = c.to_token_stream().to_string();
                if is_raw_alloc(&rp) {
                    let path = self.fn_path();
                    let name = rp.rsplit("::").next().unwrap_or_default().to_owned();
                    self.push_site(
                        Shape::Raw,
                        path,
                        format!("alloc::{name}"),
                        c.func.span(),
                        text,
                    );
                } else if let Some(k) = std_alloc_call(&rp) {
                    let path = self.fn_path();
                    self.push_site(Shape::Ctor, path, k, c.func.span(), text);
                } else if let Some(k) = self.head_ctor(&c.func) {
                    let path = self.fn_path();
                    self.push_site(Shape::Ctor, path, k, c.func.span(), text);
                } else if let Some(k) = self.ufcs_ctor(ep, &c.args) {
                    let path = self.fn_path();
                    self.push_site(Shape::Ctor, path, k, c.func.span(), text);
                }
            }
        } else {
            self.visit_expr(&c.func);
        }
        for a in &c.args {
            self.visit_expr(a);
        }
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let absorbed = self.is_absorbed(m);
        let name = m.method.to_string();
        let n_args = m.args.len();
        if self.physics {
            if name == "scope" {
                self.slice_hit("scope", m.to_token_stream().to_string(), m.method.span());
            } else if name == "try_with_active_pool" {
                self.slice_hit(
                    "try_with_active_pool",
                    m.to_token_stream().to_string(),
                    m.method.span(),
                );
            }
        }
        if !absorbed {
            let mut hit: Option<(Shape, String)> = None;
            if name == "collect" {
                hit = Some((Shape::Ctor, self.collect_container(m)));
            } else if name == "parse"
                && n_args == 0
                && let Some(h) = self.turbofish_head(m.turbofish.as_ref())
            {
                // `s.parse::<String>()` builds the head it names (tester N1).
                hit = Some((Shape::Ctor, h));
            } else if name == "clone" && n_args == 0 {
                if let Some(h) = self.clone_receiver_head(&m.receiver) {
                    hit = Some((Shape::Ctor, h));
                }
            } else if name == "spawn"
                && self.chain_root_type(&m.receiver).as_deref() == Some("Builder")
            {
                hit = Some((Shape::Ctor, "thread::spawn".to_owned()));
            } else if name == "open"
                && n_args == 1
                && self.chain_root_type(&m.receiver).as_deref() == Some("OpenOptions")
            {
                hit = Some((Shape::Ctor, "OpenOptions::open".to_owned()));
            } else if matches!(name.as_str(), "alloc" | "alloc_zeroed" | "realloc")
                && let Expr::Path(rp) = &*m.receiver
                && rp.path.is_ident("System")
            {
                hit = Some((Shape::Raw, format!("alloc::{name}")));
            } else if let Some(c) = method_ctor(&name, n_args) {
                hit = Some((Shape::Ctor, c.to_owned()));
            }
            if let Some((shape, c)) = hit {
                let path = self.fn_path();
                let text = m.to_token_stream().to_string();
                self.push_site(shape, path, c, m.method.span(), text);
            }
        }
        self.visit_expr(&m.receiver);
        if name == "with"
            && let Expr::Path(rp) = &*m.receiver
            && rp.path.segments.len() == 1
            && let Some(t) = self
                .tcx
                .tables
                .tls
                .get(&rp.path.segments[0].ident.to_string())
                .cloned()
        {
            self.pending_closure_bind = Some(t);
        }
        for a in &m.args {
            self.visit_expr(a);
        }
        self.pending_closure_bind = None;
    }

    fn visit_macro(&mut self, m: &'ast Macro) {
        self.macro_invocation(m, &[]);
    }
}

impl Visitor<'_> {
    /// The type name at the root of a receiver chain: a `X::new(..)` call or a local whose
    /// type the syntax pins.
    fn chain_root_type(&mut self, recv: &Expr) -> Option<String> {
        let root = chain_root(recv);
        if let Expr::Call(c) = root
            && let Expr::Path(p) = &*c.func
        {
            let segs: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if segs.len() >= 2 {
                return Some(segs[segs.len() - 2].clone());
            }
        }
        if let Expr::Path(p) = root
            && p.qself.is_none()
            && p.path.segments.len() == 1
            && let Some(t) = self.local_type(&format!("root:{}", p.path.segments[0].ident))
        {
            return Visitor::type_name(&t.ty);
        }
        let t = self.infer(root, 0)?;
        Visitor::type_name(&t.ty)
    }

    fn slice_hit(&mut self, which: &'static str, text: String, span: Span) {
        self.slice.push(SliceHit {
            file: self.file.to_owned(),
            item_path: self.fn_path(),
            which,
            text: clip(&text),
            line: (self.line_of)(span),
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The scan driver
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Everything one scan produced.
#[derive(Default)]
pub struct ScanOut {
    /// Sites of full-mode crates (every shape) and emitted-mode crates (`emitted` only).
    pub sites: Vec<Site>,
    /// Physics-slice hits.
    pub slice: Vec<SliceHit>,
    /// `include!` invocations: (file, argument text).
    pub includes: Vec<(String, String)>,
    /// PARSE / WALK / ALIAS-* / ORPHAN failures.
    pub failures: Vec<String>,
    /// Crate → reached file count.
    pub files_per_crate: BTreeMap<String, usize>,
    /// Every reached file.
    pub reached: BTreeSet<String>,
    /// Files excluded by a provably-false `cfg` on their `mod`, and everything under them.
    pub excluded: BTreeSet<String>,
    /// Type names defined in the scanned crates (own-head existence check).
    pub defined_types: BTreeSet<String>,
}

/// Scans every `full` and `emitted` crate of `decls` under `root`.
pub fn scan(root: &Path, decls: &Decls, vocab: &Vocab, line_of: LineOf<'_>) -> ScanOut {
    let mut out = ScanOut::default();
    let scanned = || {
        decls
            .crates
            .iter()
            .filter(|(_, (mode, _))| matches!(mode, Mode::Full | Mode::Emitted))
    };
    // Pass 1 (review W5): every scanned crate's aliases, so a path through another crate's alias
    // resolves. One crate's syntax trees are held at a time; pass 2 walks the same files again
    // and reports the walk's failures, so pass 1's are dropped here.
    let mut crates: Vec<CrateAliases> = Vec::new();
    let mut crate_idx: BTreeMap<&str, usize> = BTreeMap::new();
    for (crate_dir, _) in scanned() {
        if !crate_src(root, crate_dir).is_dir() {
            continue;
        }
        let walk = walk_crate(root, crate_dir, &mut Vec::new());
        let mut tables = Tables::default();
        for (i, f) in walk.files.iter().enumerate() {
            TableCollector {
                t: &mut tables,
                file_idx: i,
                self_ty: None,
            }
            .visit_file(&f.ast);
        }
        crate_idx.insert(crate_dir, crates.len());
        crates.push(CrateAliases {
            lib: crate_lib_name(root, crate_dir),
            aliases: tables.aliases,
            uses: walk.files.into_iter().map(|f| f.uses).collect(),
            mods: tables.mods,
        });
    }
    for (crate_dir, (mode, _)) in scanned() {
        let src = crate_src(root, crate_dir);
        if !src.is_dir() {
            out.failures.push(format!(
                "WALK  {crate_dir}  declared {} but has no src/",
                mode.name()
            ));
            continue;
        }
        let walk = walk_crate(root, crate_dir, &mut out.failures);
        out.files_per_crate
            .insert(crate_dir.clone(), walk.files.len());
        let mut tables = Tables::default();
        for (i, f) in walk.files.iter().enumerate() {
            TableCollector {
                t: &mut tables,
                file_idx: i,
                self_ty: None,
            }
            .visit_file(&f.ast);
        }
        out.defined_types.extend(tables.defined.iter().cloned());
        for f in &walk.files {
            out.reached.insert(f.rel.clone());
        }
        // Completeness (critique W3): every `.rs` under this `src/` is reached, excluded by a
        // parsed cfg, or declared unreached. A file the walk cannot see is RED, never silent.
        let mut all = Vec::new();
        all_rs_under(&src, &mut all);
        for p in all {
            let rel = rel_of(root, &p);
            let in_excluded_dir = walk
                .excluded_dirs
                .iter()
                .any(|d| rel.starts_with(&format!("{d}/")));
            if walk.excluded_files.contains(&rel) || in_excluded_dir {
                out.excluded.insert(rel);
            } else if !out.reached.contains(&rel)
                && !walk.unparsed.contains(&rel)
                && !decls.unreached.contains(&rel)
            {
                out.failures.push(format!(
                    "ORPHAN  {rel}  is under a scanned src/ but the module walk does not reach it -> declare it `unreached` with a reason, or wire it"
                ));
            }
        }
        let tcx = TypeCx {
            vocab,
            tables: &tables,
            files: &walk.files,
            crates: &crates,
            me: crate_idx
                .get(crate_dir.as_str())
                .copied()
                .expect("invariant: pass 1 indexed every scanned crate that has a src/"),
        };
        let physics = crate_dir == "crates/boyko_physics";
        for f in &walk.files {
            let mut v = Visitor {
                crate_dir,
                file: &f.rel,
                mode: *mode,
                physics,
                tcx: &tcx,
                uses: &f.uses,
                line_of,
                stack: Vec::new(),
                self_ty: Vec::new(),
                locals: vec![BTreeMap::new()],
                absorbed: BTreeSet::new(),
                pending_closure_bind: None,
                stmt_line: None,
                sites: &mut out.sites,
                slice: &mut out.slice,
                includes: &mut out.includes,
                fail: &mut out.failures,
            };
            v.visit_file(&f.ast);
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The ledger TSV
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One ledger row.
#[derive(Clone, Debug)]
pub struct Row {
    /// 1-based TSV line.
    pub line: usize,
    /// The fifteen cells, in [`TSV_HEADER`] order.
    pub cells: Vec<String>,
}

impl Row {
    /// `crate`.
    pub fn krate(&self) -> &str {
        &self.cells[0]
    }
    /// `file`.
    pub fn file(&self) -> &str {
        &self.cells[1]
    }
    /// `owner`.
    pub fn owner(&self) -> &str {
        &self.cells[2]
    }
    /// `kind`.
    pub fn kind(&self) -> &str {
        &self.cells[3]
    }
    /// `container`.
    pub fn container(&self) -> &str {
        &self.cells[4]
    }
    /// `class`.
    pub fn class(&self) -> &str {
        &self.cells[6]
    }
    /// `ecs_form`.
    pub fn form(&self) -> &str {
        &self.cells[7]
    }
    /// `owning_entity`.
    pub fn entity(&self) -> &str {
        &self.cells[8]
    }
    /// `growth`.
    pub fn growth(&self) -> &str {
        &self.cells[9]
    }
    /// `addr_cached`.
    pub fn addr_cached(&self) -> &str {
        &self.cells[10]
    }
    /// `destination`.
    pub fn destination(&self) -> &str {
        &self.cells[11]
    }
    /// `group`.
    pub fn group(&self) -> &str {
        &self.cells[12]
    }
    /// `shape`.
    pub fn shape(&self) -> &str {
        &self.cells[13]
    }
    /// `site`.
    pub fn site(&self) -> &str {
        &self.cells[14]
    }
    /// A row whose allocation happens inside std / third-party code, invisible to syntax.
    pub fn is_semantic(&self) -> bool {
        self.shape().starts_with("semantic:")
    }
    /// A kernel-storage row (M5).
    pub fn is_m5(&self) -> bool {
        self.shape() == "m5"
    }
    /// A row of a crate (or crate part) the gate does not scan.
    pub fn is_unscanned(&self) -> bool {
        self.shape() == "unscanned"
    }
    /// `ecs_form` is `out-of-scope:*`.
    pub fn is_out_of_scope(&self) -> bool {
        self.form().starts_with("out-of-scope:")
    }
    /// A row UG-02 compares against a scanned site.
    pub fn is_compared(&self) -> bool {
        !self.is_semantic() && !self.is_m5() && !self.is_unscanned()
    }
    /// A row the `ACTIVE_STD_HEAP_ROWS` pin counts: in scope and not kernel storage.
    pub fn is_std_heap(&self) -> bool {
        !self.is_out_of_scope() && !self.is_m5()
    }
}

/// Parses the ledger TSV.
pub fn parse_ledger(text: &str) -> Result<Vec<Row>, String> {
    let t = read_tsv(text, "runtime-data-ledger.tsv")?;
    expect_header(&t, TSV_HEADER, "runtime-data-ledger.tsv")?;
    Ok(t.rows
        .into_iter()
        .map(|(line, cells)| Row { line, cells })
        .collect())
}

fn check_row_vocab(r: &Row, decls: &Decls, fail: &mut Vec<String>) {
    let l = r.line;
    let mut bad = |what: &str| fail.push(format!("VOCAB  tsv:{l}  {what}"));
    if !CLASSES.contains(&r.class()) {
        bad(&format!(
            "class `{}` is not in the closed list (no U row)",
            r.class()
        ));
    }
    let form = r.form();
    let form_ok = FORMS.contains(&form)
        || form
            .strip_prefix("new-kernel-feature:")
            .is_some_and(|n| !n.trim().is_empty())
        || form
            .strip_prefix("out-of-scope:")
            .is_some_and(|t| OUT_OF_SCOPE_TAGS.contains(&t));
    if !form_ok {
        bad(&format!("ecs_form `{form}` is not in the closed list"));
    }
    if form == "out-of-scope:ruled" {
        let ruling = r
            .destination()
            .split([' ', ':', ',', ';', '('])
            .next()
            .unwrap_or_default();
        if !KEEPING_RULINGS.contains(&ruling) {
            bad(&format!(
                "out-of-scope:ruled destination must begin with a keeping ruling {KEEPING_RULINGS:?}, found `{ruling}`"
            ));
        }
    }
    if !KINDS.contains(&r.kind()) {
        bad(&format!("kind `{}`", r.kind()));
    }
    if !GROWTHS.contains(&r.growth()) {
        bad(&format!("growth `{}`", r.growth()));
    }
    if !ADDR_CACHED.contains(&r.addr_cached()) {
        bad(&format!("addr_cached `{}`", r.addr_cached()));
    }
    if !GROUPS.contains(&r.group()) {
        bad(&format!("group `{}`", r.group()));
    }
    if !OWNING_ENTITIES.contains(&r.entity()) {
        bad(&format!("owning_entity `{}`", r.entity()));
    } else if form_ok
        && r.entity() == "none"
        && !NONE_ENTITY_FORMS.contains(&form)
        && !form.starts_with("out-of-scope:")
    {
        bad(&format!(
            "owning_entity `none` is illegal for form `{form}`"
        ));
    }
    let crate_dir = crate_dir_of(r.file());
    let dir_name = crate_dir.rsplit('/').next().unwrap_or_default();
    let want_crate = if crate_dir == "." {
        "boyko-engine"
    } else {
        dir_name
    };
    if r.krate() != want_crate {
        bad(&format!(
            "crate `{}` does not match file {}",
            r.krate(),
            r.file()
        ));
    }
    let mode = decls.crates.get(crate_dir).map(|(m, _)| *m);
    let shape = r.shape();
    let is_m5_container = M5_CONTAINERS.iter().any(|p| r.container().starts_with(p));
    if shape == "m5" {
        if !is_m5_container {
            bad("shape m5 on a container that is not kernel storage");
        }
    } else if is_m5_container {
        bad("a kernel-storage container must carry shape m5");
    }
    if let Some(id) = shape.strip_prefix("semantic:") {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            bad(&format!("semantic shape id `{id}`"));
        }
        if r.site() != "-" {
            bad("a semantic row has no scanner site: `site` must be `-`");
        }
        if mode != Some(Mode::Full) {
            bad("a semantic row must be in a full-mode crate");
        }
    } else if shape == "m5" || shape == "unscanned" {
        if r.site() != "-" {
            bad(&format!(
                "a `{shape}` row has no scanner site: `site` must be `-`"
            ));
        }
        if shape == "unscanned"
            && !matches!(mode, Some(Mode::Unscanned | Mode::Pending | Mode::Emitted))
        {
            bad("shape `unscanned` in a crate the gate scans in full");
        }
    } else if let Some(sh) = Shape::from_name(shape) {
        let parts: Vec<&str> = r.site().splitn(3, '|').collect();
        if parts.len() < 3 || parts.iter().any(|p| p.is_empty()) {
            bad(&format!(
                "site `{}` is not `kind|item_path|container`",
                r.site()
            ));
        } else if parts[0] != sh.key_head() {
            bad(&format!(
                "site `{}` does not start with `{}` (shape {shape})",
                r.site(),
                sh.key_head()
            ));
        }
        match mode {
            Some(Mode::Full) if sh == Shape::Emitted => bad("shape emitted in a full-mode crate"),
            Some(Mode::Full) => {}
            Some(Mode::Emitted) if sh != Shape::Emitted => bad(&format!(
                "shape `{shape}` in an emitted-mode crate (only `emitted` or `unscanned`)"
            )),
            Some(Mode::Emitted) => {}
            _ => bad(&format!(
                "scanner shape `{shape}` in a crate that is not scanned"
            )),
        }
    } else {
        bad(&format!("shape `{shape}` is not in the closed list"));
    }
}

/// The crate directory a repository-relative file belongs to.
pub fn crate_dir_of(file: &str) -> &str {
    let parts: Vec<&str> = file.splitn(3, '/').collect();
    if parts.len() == 3 && (parts[0] == "crates" || parts[0] == "tools") {
        let end = parts[0].len() + 1 + parts[1].len();
        &file[..end]
    } else {
        "."
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Pins (`docs/memory/ledger/ug02-pins.tsv`)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// FNV-1a 64 of a pins line (the hash chain's link).
pub fn fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// One pins line.
pub struct PinLine {
    /// 1-based line in the pins file.
    pub line: usize,
    /// The raw line text.
    pub text: String,
    /// `rung`.
    pub rung: String,
    /// `pin` or `merge-raise`.
    pub kind: String,
    /// The six numeric pins, in [`PIN_NAMES`] order.
    pub vals: [i64; 6],
    /// Per-group floors.
    pub floors: BTreeMap<String, i64>,
    /// FNV-1a 64 of the previous line, or `-` on the first.
    pub prev: String,
    /// `reason`.
    pub reason: String,
}

/// The numeric pins' names, in column order.
pub const PIN_NAMES: [&str; 6] = [
    "ACTIVE_STD_HEAP_ROWS",
    "TOTAL_ACTIVE_ROWS",
    "SEMANTIC_ROWS",
    "OUT_OF_SCOPE_ROWS",
    "PHYSICS_POOL_SCOPE",
    "PHYSICS_ACTIVE_POOL",
];

/// Parses the pins file.
pub fn parse_pins(text: &str) -> Result<Vec<PinLine>, String> {
    let mut out = Vec::new();
    let mut header_seen = false;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if !header_seen {
            if c != PINS_HEADER {
                return Err(format!("ug02-pins.tsv:{}: header is {c:?}", i + 1));
            }
            header_seen = true;
            continue;
        }
        if c.len() != PINS_HEADER.len() {
            return Err(format!("ug02-pins.tsv:{}: {} cells", i + 1, c.len()));
        }
        let mut vals = [0i64; 6];
        for (k, v) in vals.iter_mut().enumerate() {
            *v = c[2 + k]
                .parse()
                .map_err(|_| format!("ug02-pins.tsv:{}: `{}` is not a count", i + 1, c[2 + k]))?;
        }
        let mut floors = BTreeMap::new();
        for part in c[8].split(';').filter(|p| !p.is_empty()) {
            let (g, n) = part
                .split_once('=')
                .ok_or_else(|| format!("ug02-pins.tsv:{}: floor `{part}`", i + 1))?;
            let n: i64 = n
                .parse()
                .map_err(|_| format!("ug02-pins.tsv:{}: floor `{part}`", i + 1))?;
            floors.insert(g.to_owned(), n);
        }
        out.push(PinLine {
            line: i + 1,
            text: line.to_owned(),
            rung: c[0].to_owned(),
            kind: c[1].to_owned(),
            vals,
            floors,
            prev: c[9].to_owned(),
            reason: c[10].to_owned(),
        });
    }
    if !header_seen {
        return Err("ug02-pins.tsv: no header".to_owned());
    }
    Ok(out)
}

/// The history rules: genesis anchored in gate code, a hash chain, unique rungs, and every
/// count non-increasing line to line except on a reasoned `merge-raise` (critique W4 (a)).
///
/// Only the first line is anchored. The chain catches an in-place edit that is not re-chained;
/// an edit that re-chains (the failure prints the hash to paste) passes, so lines 2..N are
/// append-only by the merge recipe's numstat check (PC-k), not by this function (review W1).
pub fn check_pin_history(pins: &[PinLine], genesis: Option<&str>, fail: &mut Vec<String>) {
    let mut rungs = BTreeSet::new();
    for (i, p) in pins.iter().enumerate() {
        if !rungs.insert(p.rung.as_str()) {
            fail.push(format!(
                "PIN  ug02-pins.tsv:{}  rung `{}` appears twice",
                p.line, p.rung
            ));
        }
        if p.kind != "pin" && p.kind != "merge-raise" {
            fail.push(format!("PIN  ug02-pins.tsv:{}  kind `{}`", p.line, p.kind));
        }
        if p.reason.trim().is_empty() {
            fail.push(format!("PIN  ug02-pins.tsv:{}  empty reason", p.line));
        }
        if i == 0 {
            if p.prev != "-" {
                fail.push(format!(
                    "PIN  ug02-pins.tsv:{}  the first line's prev must be `-`",
                    p.line
                ));
            }
            if let Some(g) = genesis
                && p.text != g
            {
                fail.push(format!(
                    "PIN  ug02-pins.tsv:{}  the first line differs from the genesis line anchored in gate code (the history was rewritten in place)",
                    p.line
                ));
            }
            continue;
        }
        let before = &pins[i - 1];
        if p.prev != fnv64(&before.text) {
            fail.push(format!(
                "PIN  ug02-pins.tsv:{}  prev={} but the previous line hashes to {} (history edited in place)",
                p.line,
                p.prev,
                fnv64(&before.text)
            ));
        }
        if p.kind == "merge-raise" {
            continue;
        }
        for (k, name) in PIN_NAMES.iter().enumerate() {
            if p.vals[k] > before.vals[k] {
                fail.push(format!(
                    "PIN  ug02-pins.tsv:{}  {name} rises {} -> {} on a `pin` line (only a reasoned `merge-raise` may raise)",
                    p.line, before.vals[k], p.vals[k]
                ));
            }
        }
        for (g, n) in &p.floors {
            if let Some(b) = before.floors.get(g)
                && n > b
            {
                fail.push(format!(
                    "PIN  ug02-pins.tsv:{}  floor {g} rises {b} -> {n} on a `pin` line",
                    p.line
                ));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Manifests: workspace members, package names, dependencies
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The `members` array of a `[workspace]` table.
pub fn workspace_members(root_manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_ws = false;
    let mut collecting = false;
    for raw in root_manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') && !collecting {
            in_ws = line == "[workspace]";
            continue;
        }
        if in_ws && !collecting && line.starts_with("members") && line.contains('=') {
            collecting = true;
        }
        if collecting {
            let mut rest = line;
            while let Some(a) = rest.find('"') {
                let tail = &rest[a + 1..];
                let Some(b) = tail.find('"') else { break };
                out.push(tail[..b].to_owned());
                rest = &tail[b + 1..];
            }
            if line.contains(']') {
                collecting = false;
            }
        }
    }
    out
}

fn manifest_package_name(manifest: &str) -> Option<String> {
    manifest_table_name(manifest, "[package]")
}

/// The `name = "…"` key of one manifest table (`[package]`, `[lib]`).
fn manifest_table_name(manifest: &str, table: &str) -> Option<String> {
    let mut in_table = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_table = line == table;
            continue;
        }
        if in_table
            && let Some(rest) = line.strip_prefix("name")
            && let Some(v) = rest.trim_start().strip_prefix('=')
        {
            return Some(v.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// One non-dev dependency entry.
struct Dep {
    name: String,
    package: Option<String>,
    path: bool,
    workspace: bool,
}

fn quoted_after(line: &str, key: &str) -> Option<String> {
    let i = line.find(key)?;
    let rest = line[i + key.len()..].trim_start().strip_prefix('=')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

fn bracket_delta(s: &str) -> i32 {
    let open = s.matches(['[', '{']).count();
    let close = s.matches([']', '}']).count();
    i32::try_from(open).unwrap_or(i32::MAX) - i32::try_from(close).unwrap_or(i32::MAX)
}

/// Every dependency in a `*dependencies]` section other than `dev-dependencies`.
fn manifest_deps(manifest: &str) -> Vec<Dep> {
    let mut out: Vec<Dep> = Vec::new();
    let mut section = String::new();
    let mut depth: i32 = 0;
    let mut current: Option<usize> = None;
    for raw in manifest.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if depth > 0 {
            if let Some(i) = current {
                if line.contains("path") && line.contains('=') {
                    out[i].path = true;
                }
                if let Some(p) = quoted_after(line, "package") {
                    out[i].package = Some(p);
                }
            }
            depth += bracket_delta(line);
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_owned();
            current = None;
            let is_dep_table = !section.contains("dev-dependencies")
                && !section.starts_with("workspace")
                && (section.contains("dependencies.") || section.ends_with("dependencies"));
            if is_dep_table && let Some((_, name)) = section.rsplit_once("dependencies.") {
                out.push(Dep {
                    name: name.to_owned(),
                    package: None,
                    path: false,
                    workspace: false,
                });
                current = Some(out.len() - 1);
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // `[workspace.dependencies]` pins versions for the members; it is no package's edge.
        let not_dev = !section.contains("dev-dependencies") && !section.starts_with("workspace");
        let in_dep_list = section.ends_with("dependencies") && not_dev;
        let in_dep_table = section.contains("dependencies.") && not_dev;
        if in_dep_table {
            if let Some(i) = current {
                if line.starts_with("path") {
                    out[i].path = true;
                }
                if line.starts_with("workspace") && line.contains("true") {
                    out[i].workspace = true;
                }
                if let Some(p) = quoted_after(line, "package") {
                    out[i].package = Some(p);
                }
            }
            continue;
        }
        if !in_dep_list {
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let lhs = lhs.trim();
        let (name, dotted) = match lhs.split_once('.') {
            Some((n, k)) => (n.trim(), Some(k.trim())),
            None => (lhs, None),
        };
        let workspace =
            dotted == Some("workspace") || (rhs.contains("workspace") && rhs.contains("true"));
        out.push(Dep {
            name: name.to_owned(),
            package: quoted_after(rhs, "package"),
            path: rhs.contains("path"),
            workspace,
        });
        current = Some(out.len() - 1);
        depth += bracket_delta(rhs);
    }
    out
}

/// `[workspace.dependencies]` entries that are path dependencies.
fn workspace_path_deps(root_manifest: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_ws_deps = false;
    for raw in root_manifest.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_ws_deps = line == "[workspace.dependencies]";
            continue;
        }
        if in_ws_deps
            && let Some((lhs, rhs)) = line.split_once('=')
            && rhs.contains("path")
        {
            out.insert(lhs.trim().to_owned());
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The gate
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The crate declarations, relative to the scan root.
pub const CRATES_TSV: &str = "docs/memory/ledger/ug02-crates.tsv";
/// The head-vocabulary additions.
pub const VOCAB_TSV: &str = "docs/memory/ledger/ug02-vocab.tsv";
/// The pin history.
pub const PINS_TSV: &str = "docs/memory/ledger/ug02-pins.tsv";
/// The ledger rows.
pub const LEDGER_TSV: &str = "docs/memory/runtime-data-ledger.tsv";

/// The gate's output: receipt lines, failure lines, and the counts a caller may assert on.
#[derive(Default)]
pub struct GateReport {
    /// Receipt lines (always printed).
    pub receipt: Vec<String>,
    /// One line per failure (never only the first).
    pub failures: Vec<String>,
    /// EXTRA count.
    pub extra: usize,
    /// MISSING count.
    pub missing: usize,
    /// FLOOR failure count.
    pub floor_failures: usize,
    /// Sites scanned.
    pub sites: usize,
    /// Comparison-set size.
    pub compared: usize,
    /// Files reached.
    pub files: usize,
}

impl GateReport {
    /// Receipt, failures and the RESULT line.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for l in &self.receipt {
            let _ = writeln!(s, "{l}");
        }
        for l in &self.failures {
            let _ = writeln!(s, "{l}");
        }
        if self.failures.is_empty() {
            let _ = writeln!(s, "RESULT: GREEN");
        } else {
            let _ = writeln!(s, "RESULT: RED ({} failures)", self.failures.len());
        }
        s
    }

    /// Failure lines starting with `tag`.
    pub fn with_tag(&self, tag: &str) -> Vec<&String> {
        self.failures
            .iter()
            .filter(|f| f.starts_with(tag))
            .collect()
    }
}

fn read_data(root: &Path, rel: &str, fail: &mut Vec<String>) -> Option<String> {
    match fs::read_to_string(root.join(rel)) {
        Ok(t) => Some(t),
        Err(e) => {
            fail.push(format!("DATA  {rel}  cannot read: {e}"));
            None
        }
    }
}

fn check_members(
    root_manifest: &str,
    decls: &Decls,
    baseline: &[(&str, Mode)],
    fail: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut members: BTreeSet<String> = workspace_members(root_manifest).into_iter().collect();
    if manifest_package_name(root_manifest).is_some() {
        members.insert(".".to_owned());
    }
    for m in &members {
        if !decls.crates.contains_key(m) {
            fail.push(format!(
                "MEMBER  {m}  is a workspace member with no ug02-crates.tsv line -> declare its mode"
            ));
        }
    }
    for (path, (mode, _)) in &decls.crates {
        let is_member = members.contains(path);
        match mode {
            Mode::Pending if is_member => fail.push(format!(
                "MEMBER  {path}  is declared pending but is a workspace member -> reclassify it (W5)"
            )),
            Mode::Pending => {}
            _ if !is_member => {
                fail.push(format!("MEMBER  {path}  is declared but is not a workspace member"));
            }
            _ => {}
        }
    }
    for (path, mode) in baseline {
        match decls.crates.get(*path) {
            Some((m, _)) if m == mode => {}
            Some((m, _)) => fail.push(format!(
                "MEMBER  {path}  must stay `{}` (gate baseline), declared `{}`",
                mode.name(),
                m.name()
            )),
            None => fail.push(format!(
                "MEMBER  {path}  (gate baseline, `{}`) is not declared",
                mode.name()
            )),
        }
    }
    members
}

fn check_deps(
    root: &Path,
    root_manifest: &str,
    members: &BTreeSet<String>,
    decls: &Decls,
    fail: &mut Vec<String>,
) {
    let manifest_of = |m: &str| {
        if m == "." {
            root.join("Cargo.toml")
        } else {
            root.join(m).join("Cargo.toml")
        }
    };
    let mut pkg_to_path: BTreeMap<String, String> = BTreeMap::new();
    for m in members {
        if let Ok(t) = fs::read_to_string(manifest_of(m))
            && let Some(n) = manifest_package_name(&t)
        {
            pkg_to_path.insert(n, m.clone());
        }
    }
    let ws_path_deps = workspace_path_deps(root_manifest);
    let mut seen_third: BTreeSet<(String, String)> = BTreeSet::new();
    for (path, (mode, _)) in &decls.crates {
        if !matches!(mode, Mode::Full | Mode::Emitted) || !members.contains(path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(manifest_of(path)) else {
            continue;
        };
        for d in manifest_deps(&text) {
            let pkg = d.package.clone().unwrap_or_else(|| d.name.clone());
            let local = d.path || (d.workspace && ws_path_deps.contains(&d.name));
            if local {
                let Some(target) = pkg_to_path.get(&pkg) else {
                    fail.push(format!(
                        "DEP  {path}  depends on path package `{pkg}` that is no workspace member"
                    ));
                    continue;
                };
                if let Some((tm, _)) = decls.crates.get(target)
                    && matches!(tm, Mode::Unscanned | Mode::Pending)
                {
                    fail.push(format!(
                        "DEP  {path}  ({}) depends on `{pkg}` ({target}, {}) -> scan it or drop the edge",
                        mode.name(),
                        tm.name()
                    ));
                }
            } else if *mode == Mode::Full {
                let key = (path.clone(), d.name.clone());
                seen_third.insert(key.clone());
                if !decls.third_party.contains_key(&key) {
                    fail.push(format!(
                        "THIRDPARTY  {path}  depends on third-party `{}` with no ug02-crates.tsv `third-party` line -> classify the heap types it contributes",
                        d.name
                    ));
                }
            }
        }
    }
    for key in decls.third_party.keys() {
        if !seen_third.contains(key) {
            fail.push(format!(
                "THIRDPARTY  {}  declares `{}`, which is no longer a dependency",
                key.0, key.1
            ));
        }
    }
}

/// What one gate run reads, and the anchors that live in gate code.
pub struct GateConfig<'a> {
    /// Where the ledger and the `ug02-*.tsv` files are read.
    pub data_root: &'a Path,
    /// Where the manifests and the source trees are read (the data root, except for the
    /// empty-root control).
    pub scan_root: &'a Path,
    /// The first pins line, anchored in gate code (critique W4 (a)); `None` in fixtures.
    pub genesis: Option<&'a str>,
    /// Crates that must stay scanned in the given mode ([`BASELINE_SCANNED`] for the real tree).
    pub baseline: &'a [(&'a str, Mode)],
    /// The line provider.
    pub line_of: LineOf<'a>,
}

/// Runs UG-02.
pub fn run_gate(cfg: &GateConfig<'_>) -> GateReport {
    let data_root = cfg.data_root;
    let root = cfg.scan_root;
    let genesis = cfg.genesis;
    let line_of = cfg.line_of;
    let mut rep = GateReport::default();
    rep.receipt
        .push(format!("UG-02 runtime data ledger @ {}", root.display()));
    let mut fail: Vec<String> = Vec::new();

    let decls = match read_data(data_root, CRATES_TSV, &mut fail).map(|t| parse_decls(&t)) {
        Some(Ok(d)) => d,
        Some(Err(e)) => {
            fail.push(format!("DATA  {e}"));
            Decls::default()
        }
        None => Decls::default(),
    };
    let vocab = match read_data(data_root, VOCAB_TSV, &mut fail).map(|t| Vocab::with_additions(&t))
    {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            fail.push(format!("DATA  {e}"));
            Vocab::baseline()
        }
        None => Vocab::baseline(),
    };
    let rows = match read_data(data_root, LEDGER_TSV, &mut fail).map(|t| parse_ledger(&t)) {
        Some(Ok(r)) => r,
        Some(Err(e)) => {
            fail.push(format!("DATA  {e}"));
            Vec::new()
        }
        None => Vec::new(),
    };
    let pins = match read_data(data_root, PINS_TSV, &mut fail).map(|t| parse_pins(&t)) {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => {
            fail.push(format!("DATA  {e}"));
            None
        }
        None => None,
    };

    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_else(|e| {
        fail.push(format!(
            "MEMBER  Cargo.toml  cannot read the root manifest: {e}"
        ));
        String::new()
    });
    let members = check_members(&root_manifest, &decls, cfg.baseline, &mut fail);
    check_deps(root, &root_manifest, &members, &decls, &mut fail);

    let scan_out = scan(root, &decls, &vocab, line_of);
    fail.extend(scan_out.failures.iter().cloned());
    for (h, o) in &vocab.heads {
        if *o == Origin::Own && !scan_out.defined_types.contains(h) {
            fail.push(format!(
                "VOCAB  own head `{h}` names no type defined in a scanned crate"
            ));
        }
    }
    let found_includes: BTreeSet<(String, String)> = scan_out.includes.iter().cloned().collect();
    for inc in found_includes.difference(&decls.includes) {
        fail.push(format!(
            "INCLUDE  {}  include!({}) is not declared -> the walk cannot see the included code",
            inc.0, inc.1
        ));
    }
    for inc in decls.includes.difference(&found_includes) {
        fail.push(format!(
            "INCLUDE  {}  declared include!({}) no longer exists",
            inc.0, inc.1
        ));
    }
    for (path, (mode, _)) in &decls.crates {
        if matches!(mode, Mode::Full | Mode::Emitted)
            && members.contains(path)
            && scan_out.files_per_crate.get(path).copied().unwrap_or(0) == 0
        {
            fail.push(format!(
                "CRATE  {path}  is scanned `{}` but no file was parsed",
                mode.name()
            ));
        }
    }

    for r in &rows {
        check_row_vocab(r, &decls, &mut fail);
    }
    // P6 (ii): every scanned crate with compared rows yields at least one site.
    let crates_with_sites: BTreeSet<&str> = scan_out
        .sites
        .iter()
        .map(|s| s.crate_dir.as_str())
        .collect();
    let crates_with_rows: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.is_compared())
        .map(|r| crate_dir_of(r.file()))
        .collect();
    for c in crates_with_rows.difference(&crates_with_sites) {
        fail.push(format!(
            "CRATE  {c}  has compared rows but the scan found no site in it"
        ));
    }
    let mut excluded_rows = 0usize;
    for r in rows.iter().filter(|r| r.is_compared()) {
        if scan_out.reached.contains(r.file()) {
            continue;
        }
        let in_excluded = scan_out.excluded.contains(r.file());
        if in_excluded {
            excluded_rows += 1;
        }
        fail.push(format!(
            "UNREACHED  tsv:{}  {}  the row claims a site in a file the walk does not scan{}",
            r.line,
            r.file(),
            if in_excluded {
                " (a cfg-excluded file)"
            } else {
                ""
            }
        ));
    }

    // Exact multiset comparison over (file, site).
    let mut scan_ms: BTreeMap<(String, String), Vec<&Site>> = BTreeMap::new();
    for s in &scan_out.sites {
        scan_ms
            .entry((s.file.clone(), s.key()))
            .or_default()
            .push(s);
    }
    let mut row_ms: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.is_compared()) {
        row_ms
            .entry((r.file().to_owned(), r.site().to_owned()))
            .or_default()
            .push(r);
    }
    let mut matched_by_group: BTreeMap<String, i64> = BTreeMap::new();
    let mut compared_by_group: BTreeMap<String, i64> = BTreeMap::new();
    for ((file, key), rs) in &row_ms {
        let n_sites = scan_ms
            .get(&(file.clone(), key.clone()))
            .map_or(0, Vec::len);
        for (i, r) in rs.iter().enumerate() {
            *compared_by_group.entry(r.group().to_owned()).or_default() += 1;
            if i < n_sites {
                *matched_by_group.entry(r.group().to_owned()).or_default() += 1;
            } else {
                rep.missing += 1;
                fail.push(format!(
                    "MISSING  tsv:{}  {}  {}  owner=\"{}\"  ({} rows, {} sites) -> retire the row and lower the pins, or restore the site",
                    r.line,
                    file,
                    key,
                    r.owner(),
                    rs.len(),
                    n_sites
                ));
            }
        }
    }
    for ((file, key), ss) in &scan_ms {
        let n_rows = row_ms.get(&(file.clone(), key.clone())).map_or(0, Vec::len);
        for s in ss.iter().skip(n_rows) {
            rep.extra += 1;
            let line = s
                .line
                .or(s.stmt_line)
                .map(|l| format!(":{l}"))
                .unwrap_or_default();
            fail.push(format!(
                "EXTRA  {file}{line}  {key}  \"{}\"  ({} sites, {} rows) -> add a row or remove the site",
                s.text,
                ss.len(),
                n_rows
            ));
        }
    }

    // Receipt counts.
    let mut by_shape: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &scan_out.sites {
        *by_shape.entry(s.shape.name()).or_default() += 1;
    }
    let n_mode = |m: Mode| decls.crates.values().filter(|(x, _)| *x == m).count();
    let pending: Vec<String> = decls
        .crates
        .iter()
        .filter(|(_, (m, _))| *m == Mode::Pending)
        .map(|(p, (_, r))| format!("{p}: {r}"))
        .collect();
    rep.files = scan_out.files_per_crate.values().sum();
    rep.sites = scan_out.sites.len();
    rep.receipt.push(format!(
        "scan: full {} crates / emitted {} / unscanned {} / pending {} ({}); files {}; excluded files {}; sites {} ({})",
        n_mode(Mode::Full),
        n_mode(Mode::Emitted),
        n_mode(Mode::Unscanned),
        n_mode(Mode::Pending),
        pending.join("; "),
        rep.files,
        scan_out.excluded.len(),
        rep.sites,
        Shape::ALL
            .iter()
            .map(|s| format!("{} {}", s.name(), by_shape.get(s.name()).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let count = |f: &dyn Fn(&Row) -> bool| {
        i64::try_from(rows.iter().filter(|r| f(r)).count()).unwrap_or(i64::MAX)
    };
    let total = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    let std_heap = count(&|r| r.is_std_heap());
    let semantic = count(&|r| r.is_semantic());
    let oos = count(&|r| r.is_out_of_scope());
    let m5 = count(&|r| r.is_m5());
    let unscanned = count(&|r| r.is_unscanned());
    rep.compared = row_ms.values().map(Vec::len).sum();
    rep.receipt.push(format!(
        "ledger: rows {total} (in scope {}, out of scope {oos}, M5 {m5}, semantic {semantic}, unscanned {unscanned}); comparison set {}; rows in cfg-excluded files {excluded_rows}",
        total - oos,
        rep.compared
    ));
    rep.receipt.push(format!(
        "exact: extra {}, missing {}",
        rep.extra, rep.missing
    ));

    let n_scope =
        i64::try_from(scan_out.slice.iter().filter(|h| h.which == "scope").count()).unwrap_or(0);
    let n_active = i64::try_from(
        scan_out
            .slice
            .iter()
            .filter(|h| h.which == "try_with_active_pool")
            .count(),
    )
    .unwrap_or(0);
    let counts = [std_heap, total, semantic, oos, n_scope, n_active];
    if let Some(pins) = &pins {
        match pins.last() {
            None => fail.push("PIN  ug02-pins.tsv has no pin line".to_owned()),
            Some(last) => {
                for (k, name) in PIN_NAMES.iter().enumerate() {
                    if last.vals[k] != counts[k] {
                        fail.push(format!(
                            "PIN  {name} pinned={} ledger={}",
                            last.vals[k], counts[k]
                        ));
                    }
                }
                let floor_groups: BTreeSet<&str> = last.floors.keys().map(String::as_str).collect();
                if floor_groups != GROUPS.iter().copied().collect::<BTreeSet<&str>>() {
                    fail.push(format!(
                        "PIN  floors must name exactly the 13 groups, found {floor_groups:?}"
                    ));
                }
                for g in GROUPS {
                    let floor = last.floors.get(*g).copied().unwrap_or(0);
                    let cmp = compared_by_group.get(*g).copied().unwrap_or(0);
                    let scanned = matched_by_group.get(*g).copied().unwrap_or(0);
                    let cmp_sign = if scanned < floor { "<" } else { ">=" };
                    rep.receipt.push(format!(
                        "floors: {g} scanned {scanned} {cmp_sign} floor {floor} (comparison set {cmp})"
                    ));
                    // A floor of 0 is legal exactly when the group has no compared row (review
                    // W2: the rung that retires a group's last row, e.g. macros-aether's KF-43
                    // rows at D-E13, pins 0). `floor == cmp` makes a 0 on a group that still has
                    // rows RED; an empty scan of a scanned crate is RED per crate (CRATE).
                    if floor != cmp {
                        fail.push(format!(
                            "PIN  floor {g}={floor} but the group's comparison set is {cmp}"
                        ));
                    }
                    if scanned < floor {
                        rep.floor_failures += 1;
                        fail.push(format!("FLOOR  {g} scanned={scanned} < floor={floor}"));
                    }
                }
                rep.receipt.push(format!(
                    "pins: {}; history {} lines, raises {}",
                    PIN_NAMES
                        .iter()
                        .enumerate()
                        .map(|(k, n)| format!("{n} pin={} ledger={}", last.vals[k], counts[k]))
                        .collect::<Vec<_>>()
                        .join("; "),
                    pins.len(),
                    pins.iter().filter(|p| p.kind == "merge-raise").count()
                ));
            }
        }
        check_pin_history(pins, genesis, &mut fail);
    }
    let slice_list = |which: &str| {
        scan_out
            .slice
            .iter()
            .filter(|h| h.which == which)
            .map(|h| {
                let line = h.line.map(|l| format!(":{l}")).unwrap_or_default();
                format!("{}{line} {} `{}`", h.file, h.item_path, h.text)
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    rep.receipt.push(format!(
        "physics slice: pool.scope {n_scope} [{}]",
        slice_list("scope")
    ));
    rep.receipt.push(format!(
        "physics slice: try_with_active_pool {n_active} [{}]",
        slice_list("try_with_active_pool")
    ));
    rep.receipt.push(format!(
        "include!: {} declared [{}]",
        decls.includes.len(),
        decls
            .includes
            .iter()
            .map(|(f, a)| format!("{f} {a}"))
            .collect::<Vec<_>>()
            .join("; ")
    ));
    rep.failures = fail;
    rep
}
