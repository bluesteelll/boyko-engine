//! **G-LOOP (plan gate UG-14): every place the windowed runner's frame loop writes the World
//! outside a system, taken from the AST of `src/runner.rs`.**
//!
//! # What is measured
//!
//! `fn frame_loop` in `src/runner.rs` is the per-frame host loop. Everything it does to the World
//! between two `Schedule::run`s is a write no system declares, so the scheduler cannot order it,
//! the change-detection tick cannot attribute it, and G-GRAPH cannot see it. The engine design
//! (`ENGINE-RUNTIME-ECS-DESIGN.md` §12, G-LOOP) wants that set to shrink to nothing: HO2 takes two
//! steps into Main systems, HO4 takes the rest into the render schedule.
//!
//! This census lands the gate **at today's counts** (design K0). On this tree the loop holds
//! **18 World-write sites over 9 host steps** (F3, F7, F8, F9, F10, F11, F12, F14, F15), plus the
//! EXEMPT F2 (the OS input pump, which stays host code by the research ruling) and the one
//! SCHEDULER site F4 (`guarded_update` → `App::update_with_delta`). The design's row 120 says 7
//! steps; F9 (`MaterialTable::take_rebind_pending`) and F14 (the particle readback's two
//! `insert_resource`s) were already present on the tree the design was written against
//! (`d552be05`), so that count was short, not the tree drifted. HO4's target of 0 still holds: R3
//! absorbs F8 and F9, R7 absorbs F14.
//!
//! # How — a structural walk, not a grep
//!
//! The file is parsed with [`syn::parse_file`] and `frame_loop`'s body is walked with
//! [`syn::visit::Visit`]. Every occurrence of the `app` binding (and of every `&World` value the
//! walk derives from it) must fall into exactly one class, and **anything else is RED** — the walk
//! fails closed:
//!
//! * **(W1) write site** — a method chain rooted at `app.world_mut()`. One acquisition is one
//!   site; everything done later through the obtained `&mut` belongs to it. Keyed by the printed
//!   chain (arguments kept, no line numbers).
//! * **(W2) method on `app`** — only `world` (a read, see W4), `world_mut` (W1) and
//!   `update_with_delta` (SCHEDULER) are classified. Any other method is RED, whatever its
//!   receiver type. The `&mut self` methods of EVERY `impl App` block (both files under
//!   `boyko_ecs/src/ecs/core/app/`) are printed so a reader sees what the rule shuts out —
//!   `add_plugins` lives in `plugins.rs`, not `app.rs`, and an `X::build` can insert resources.
//! * **(W3) `app` passed to a function** — resolved against `runner.rs`'s own top-level fns. A
//!   `&mut App` or `&App` parameter makes the walk descend into the callee with that parameter as
//!   the new `app`; the call site takes the callee's class (a write anywhere inside makes it a
//!   write site; `update_with_delta` alone makes it SCHEDULER; reads alone make it a read). A
//!   callee not defined in `runner.rs`, a qualified callee path, a method callee, or any other
//!   parameter type is RED.
//! * **(W4) `&World` reads** — `app.world()` must be either the receiver of a method in
//!   [`READ_METHODS`] or the whole initialiser of a `let <ident> = app.world();`, and every use of
//!   such a binding must be the receiver of a [`READ_METHODS`] method. A `&World` passed anywhere
//!   else — `helper(app.world())`, `helper(world)`, `&app.world()` — is RED: `EcsMaster` has
//!   `&self` methods that write (`send_event` writes event lanes through `&self`), so a `&World`
//!   the walk cannot follow is an unclassified write. [`READ_METHODS`] is checked against the
//!   `impl EcsMaster` blocks: each must take `&self` and return no `&mut`/`Mut<`.
//! * **(W5) macros** — a macro whose tokens name `app` or a tracked `&World` binding must parse as
//!   a comma-separated expression list, and is then walked like any other code. One that does not
//!   parse is RED.
//! * **Anything else** — `(*app).world_mut()`, `let a = &mut *app;`, `Foo { app }`, a `let` or
//!   closure parameter that shadows `app` or a tracked binding, a nested item that names `app` —
//!   is RED.
//!
//! # Pins and their rule
//!
//! [`ALLOW`] holds one row per `(step, class, key)` with its count. The detected multiset must
//! equal it **exactly**: an extra site is RED (unlisted), a vanished site is RED until its row
//! and the pins are lowered in the same commit, and a pin that is raised is a finding.
//! [`G_LOOP_STEPS`] = 9 and [`G_LOOP_SITES`] = 18 are derived from [`ALLOW`]'s COUNTED rows and
//! asserted equal to it, so the two cannot disagree. **Shrink-only, target 0** (HO2 −2: F11 and
//! F12; HO4 → 0).
//!
//! # Red controls — permanent, in this binary
//!
//! [`walk_fails_closed_on_every_unclassified_shape`] clones the parsed `frame_loop`, inserts one
//! statement at the top of its `loop` body, re-walks, and requires RED for each of: a new
//! `world_mut` write (i); the same write inside `debug_assert!` (ii); a local `fn poke(app: &mut
//! App)` called with `app` (iii) and a callee that is not in the file (iii-b); `app.add_plugins`
//! (v); a `&World` passed to a helper that calls `send_event` (vi) and the same through a `let`
//! binding (vii); `(*app).world_mut()` (viii); `let a = &mut *app` (ix); `send_event` on a tracked
//! `&World` binding (x); a `let app` shadow (xi). The GREEN control (iv) adds a
//! `app.world().resource::<WindowInfo>()` read and requires the site multiset unchanged.
//!
//! # Blind spots, stated
//!
//! * **Interior mutability.** A write made through the `&T` a READ method returns (a `Cell`, an
//!   atomic, an `UnsafeCell` inside a resource) is invisible to a syntax walk. A read is classified
//!   by the method that produced it, not by what is done with the value afterwards.
//! * **The World is reached only through `app`.** `frame_loop`'s signature is asserted to be
//!   exactly `(app: &mut App, host: &mut WindowHost, ctx: &'static VulkanContext)`, and
//!   `WindowHost`'s direct field types are asserted not to name `App`/`EcsMaster`. A World handle
//!   buried deeper inside a host field, a thread-local or a global would escape this walk.
//! * **Callees outside `runner.rs`** are RED rather than followed, so a helper moved to another
//!   module turns this gate red until the walk is taught to resolve it.
//!
//! No wall-clock figure is produced anywhere in this binary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Block, Expr, ExprCall, ExprClosure, ExprMethodCall, ExprPath, FnArg, ImplItem, Item, ItemFn,
    Local, Macro, Pat, Stmt, Token, Type,
};

// ═══════════════════════════════════════════════════════════════════════════
// The committed allowlist and its pins
// ═══════════════════════════════════════════════════════════════════════════

/// What one detected site is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// A World write outside a system — the G-LOOP count.
    Counted,
    /// A World write outside a system that stays host code by ruling (F2, the OS input pump:
    /// `ENGINE-RUNTIME-ECS-RESEARCH.md` "The OS pump (F1/F2) stays host code").
    Exempt,
    /// The one scheduled step: the frame's `Schedule::run`s (F4).
    Scheduler,
}

/// The kind the WALK can tell apart. `Counted` and `Exempt` are both writes; which one a write is
/// is the allowlist's ruling, not the walk's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Write,
    Scheduler,
}

impl Class {
    const fn kind(self) -> Kind {
        match self {
            Class::Counted | Class::Exempt => Kind::Write,
            Class::Scheduler => Kind::Scheduler,
        }
    }
}

/// The allowlist: `(host step, class, site key, count)`. The key is the printed call chain with
/// its arguments; the steps are the host lens's (`ENGINE-RUNTIME-ECS-RESEARCH.md` §1). Measured on
/// `u/b2` @ `c1e9f1db`.
const ALLOW: &[(&str, Class, &str, u32)] = &[
    (
        "F2",
        Class::Exempt,
        "app.world_mut().resource_mut::<RawInputQueue>()",
        1,
    ),
    (
        "F3",
        Class::Counted,
        "app.world_mut().resource_mut::<RenderEpoch>()",
        1,
    ),
    (
        "F4",
        Class::Scheduler,
        "guarded_update(app,dt,\"frame\")",
        1,
    ),
    (
        "F7",
        Class::Counted,
        "retire_deferred_frees(#0: app.world_mut())",
        1,
    ),
    (
        "F8",
        Class::Counted,
        "app.world_mut().remove_non_send_resource::<RetiredGpuBuffers>().expect(\"invariant: RetiredGpuBuffers inserted at boot\")",
        1,
    ),
    (
        "F8",
        Class::Counted,
        "app.world_mut().remove_non_send_resource::<MaterialTable>().expect(\"invariant: MaterialTable inserted at boot\")",
        1,
    ),
    (
        "F8",
        Class::Counted,
        "app.world_mut().insert_non_send_resource(material_table)",
        1,
    ),
    (
        "F8",
        Class::Counted,
        "app.world_mut().insert_non_send_resource(retired)",
        1,
    ),
    (
        "F9",
        Class::Counted,
        "app.world_mut().non_send_resource_mut::<MaterialTable>().take_rebind_pending(s)",
        1,
    ),
    (
        "F10",
        Class::Counted,
        "app.world_mut().resource_mut::<SdfEditStaging>()",
        1,
    ),
    (
        "F11",
        Class::Counted,
        "app.world_mut().try_resource_mut::<TaaState>()",
        1,
    ),
    (
        "F11",
        Class::Counted,
        "app.world_mut().try_resource_mut::<JitterState>()",
        1,
    ),
    (
        "F11",
        Class::Counted,
        // The three `cfg` arms (hwrt MV camera, hwrt TAA fallback, not(hwrt)) each hand the
        // advanced pair to `Some(..)`, so the key carries the constructor it is an argument of.
        "Some(#0: app.world_mut().resource_mut::<boyko_render::MotionCamState>().advance(cur))",
        3,
    ),
    (
        "F12",
        Class::Counted,
        "guarded_run_system(app,\"vb-instance-ring\",boyko_render::sync_vb_instance_ring_system)",
        1,
    ),
    (
        "F14",
        Class::Counted,
        "app.world_mut().insert_resource(sort_readback)",
        1,
    ),
    (
        "F14",
        Class::Counted,
        "app.world_mut().insert_resource(readback)",
        1,
    ),
    (
        "F15",
        Class::Counted,
        "app.world_mut().resource_mut::<WindowInfo>()",
        1,
    ),
    (
        "F15",
        Class::Counted,
        "app.world_mut().resource_mut::<HostFrameStats>()",
        1,
    ),
];

/// Host steps that write the World outside a system. Shrink-only; target 0 (HO2 −2, HO4 → 0).
/// The design's row 120 says 7; this tree measures 9 (F9 and F14 were uncounted there).
const G_LOOP_STEPS: usize = 9;
/// World-write sites over those steps. Shrink-only; target 0.
const G_LOOP_SITES: usize = 18;

/// The `&World` methods a read may call. Today's loop uses exactly these four; each is checked
/// against its `impl EcsMaster` signature by [`read_methods_are_shared_and_non_mut`].
const READ_METHODS: &[&str] = &[
    "resource",
    "try_resource",
    "non_send_resource",
    "contains_resource",
];

/// The methods the walk classifies on `app` itself. Everything else on `app` is RED.
const APP_READ: &str = "world";
const APP_WRITE: &str = "world_mut";
const APP_SCHEDULER: &str = "update_with_delta";

/// Callee recursion bound: `frame_loop` → `guarded_run_system` is depth 1 today.
const MAX_DEPTH: u32 = 8;

// ═══════════════════════════════════════════════════════════════════════════
// Sources
// ═══════════════════════════════════════════════════════════════════════════

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn parse(path: &Path) -> syn::File {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("G-LOOP: cannot read {}: {e}", path.display()));
    syn::parse_file(&src).unwrap_or_else(|e| panic!("G-LOOP: cannot parse {}: {e}", path.display()))
}

/// Every `.rs` file directly inside `dir`, sorted, parsed.
fn parse_dir(dir: &Path) -> Vec<(PathBuf, syn::File)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("G-LOOP: cannot list {}: {e}", dir.display()))
        .map(|e| {
            e.expect("invariant: a listed directory entry is readable")
                .path()
        })
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let f = parse(&p);
            (p, f)
        })
        .collect()
}

/// `runner.rs`'s top-level fns by name (a name can repeat across `cfg` arms).
fn fn_table(file: &syn::File) -> BTreeMap<String, Vec<ItemFn>> {
    let mut t: BTreeMap<String, Vec<ItemFn>> = BTreeMap::new();
    for item in &file.items {
        if let Item::Fn(f) = item {
            t.entry(f.sig.ident.to_string())
                .or_default()
                .push(f.clone());
        }
    }
    t
}

// ═══════════════════════════════════════════════════════════════════════════
// Keys — printed tokens, no whitespace dependence, no line numbers
// ═══════════════════════════════════════════════════════════════════════════

/// Prints tokens compactly: a space only between two adjacent words (ident / literal), literals
/// verbatim. A rustfmt reflow cannot move a key; an edit to the chain or its arguments does.
fn key_of(tokens: TokenStream) -> String {
    fn push(ts: TokenStream, out: &mut String, prev_word: &mut bool) {
        for tt in ts {
            match tt {
                TokenTree::Ident(i) => {
                    if *prev_word {
                        out.push(' ');
                    }
                    out.push_str(&i.to_string());
                    *prev_word = true;
                }
                TokenTree::Literal(l) => {
                    if *prev_word {
                        out.push(' ');
                    }
                    out.push_str(&l.to_string());
                    *prev_word = true;
                }
                TokenTree::Punct(p) => {
                    out.push(p.as_char());
                    *prev_word = false;
                }
                TokenTree::Group(g) => {
                    let (open, close) = match g.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => ("", ""),
                    };
                    out.push_str(open);
                    *prev_word = false;
                    push(g.stream(), out, prev_word);
                    out.push_str(close);
                    *prev_word = false;
                }
            }
        }
    }
    let mut out = String::new();
    let mut prev_word = false;
    push(tokens, &mut out, &mut prev_word);
    out
}

fn key<T: ToTokens>(node: &T) -> String {
    key_of(node.to_token_stream())
}

/// A shortened key for RED messages.
fn short<T: ToTokens>(node: &T) -> String {
    let mut k = key(node);
    if k.len() > 160 {
        let mut cut = 160;
        while !k.is_char_boundary(cut) {
            cut -= 1;
        }
        k.truncate(cut);
        k.push('…');
    }
    k
}

// ═══════════════════════════════════════════════════════════════════════════
// The walk
// ═══════════════════════════════════════════════════════════════════════════

/// What one walked body contains.
#[derive(Default, Debug)]
struct Walk {
    /// Every write / scheduler site, keyed.
    sites: Vec<(Kind, String)>,
    /// `&World` reads (classified, not pinned).
    reads: usize,
    /// Every shape the walk refused to classify. Non-empty = RED.
    reds: Vec<String>,
}

/// If `e` (after `(..)`) is the path `name`, returns it.
fn as_ident_path<'e>(e: &'e Expr, name: &str) -> Option<&'e ExprPath> {
    match e {
        Expr::Paren(p) => as_ident_path(&p.expr, name),
        Expr::Path(p) if p.qself.is_none() && p.path.is_ident(name) => Some(p),
        _ => None,
    }
}

/// Is `e` exactly `<app>.<method>()`?
fn app_call<'e>(e: &'e Expr, app: &str, method: &str) -> Option<&'e ExprMethodCall> {
    match e {
        Expr::Paren(p) => app_call(&p.expr, app, method),
        Expr::MethodCall(m) if m.method == method && as_ident_path(&m.receiver, app).is_some() => {
            Some(m)
        }
        _ => None,
    }
}

/// The next link down a receiver chain.
fn chain_next(e: &Expr) -> Option<&Expr> {
    match e {
        Expr::MethodCall(m) => Some(&m.receiver),
        Expr::Try(t) => Some(&t.expr),
        Expr::Paren(p) => Some(&p.expr),
        Expr::Field(f) => Some(&f.base),
        _ => None,
    }
}

/// Strips `&`, `&mut`, `*` and `(..)` off an argument.
fn strip_ref(e: &Expr) -> &Expr {
    match e {
        Expr::Reference(r) => strip_ref(&r.expr),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => strip_ref(&u.expr),
        Expr::Paren(p) => strip_ref(&p.expr),
        _ => e,
    }
}

/// Is `t` `&mut App` / `&App` (any lifetime)? `Some(true)` for `&mut`.
fn app_ref_type(t: &Type) -> Option<bool> {
    if let Type::Reference(r) = t
        && let Type::Path(p) = &*r.elem
        && p.qself.is_none()
        && p.path
            .segments
            .last()
            .is_some_and(|s| s.ident == "App" && s.arguments.is_empty())
    {
        return Some(r.mutability.is_some());
    }
    None
}

/// Does `ts` contain an identifier in `names`, at any depth?
fn tokens_name_any(ts: TokenStream, names: &[String]) -> bool {
    ts.into_iter().any(|tt| match tt {
        TokenTree::Ident(i) => names.iter().any(|n| i == n.as_str()),
        TokenTree::Group(g) => tokens_name_any(g.stream(), names),
        _ => false,
    })
}

/// Every identifier a pattern binds.
fn pat_idents(p: &Pat, out: &mut Vec<String>) {
    struct V<'o>(&'o mut Vec<String>);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
            self.0.push(i.ident.to_string());
            syn::visit::visit_pat_ident(self, i);
        }
    }
    V(out).visit_pat(p);
}

struct Walker<'t> {
    fns: &'t BTreeMap<String, Vec<ItemFn>>,
    /// The `App` binding's name in the body being walked.
    app: String,
    /// Names bound by `let <ident> = app.world();`.
    worlds: Vec<String>,
    /// Addresses of nodes a parent has already classified.
    claimed: Vec<usize>,
    depth: u32,
    out: Walk,
    /// The statement being walked, for RED messages.
    stmt: String,
}

fn addr<T>(n: &T) -> usize {
    std::ptr::from_ref(n) as usize
}

impl<'t> Walker<'t> {
    fn new(fns: &'t BTreeMap<String, Vec<ItemFn>>, app: String, depth: u32) -> Self {
        Walker {
            fns,
            app,
            worlds: Vec::new(),
            claimed: Vec::new(),
            depth,
            out: Walk::default(),
            stmt: String::new(),
        }
    }

    fn claim<T>(&mut self, n: &T) {
        self.claimed.push(addr(n));
    }

    fn is_claimed<T>(&self, n: &T) -> bool {
        self.claimed.contains(&addr(n))
    }

    fn red(&mut self, what: String) {
        let at = self.stmt.clone();
        self.out.reds.push(format!("{what}   [in: {at}]"));
    }

    /// Is `e` a tracked `&World`: `app.world()` or a binding of it?
    fn world_value<'e>(&self, e: &'e Expr) -> Option<&'e Expr> {
        let e = match e {
            Expr::Paren(p) => &p.expr,
            _ => e,
        };
        if app_call(e, &self.app, APP_READ).is_some() {
            return Some(e);
        }
        if let Expr::Path(p) = e
            && p.qself.is_none()
            && self.worlds.iter().any(|w| p.path.is_ident(w.as_str()))
        {
            return Some(e);
        }
        None
    }

    /// Claims `e` and, if it is `app.world()`, its `app` receiver.
    fn claim_world_value(&mut self, e: &Expr) {
        self.claim(e);
        if let Expr::MethodCall(m) = e {
            self.claim(m);
            self.claim(&*m.receiver);
            if let Expr::Paren(p) = &*m.receiver {
                self.claim(&*p.expr);
            }
        }
        if let Expr::Paren(p) = e {
            self.claim(&*p.expr);
        }
    }

    /// If `e` is a receiver chain rooted at `app.world_mut()`, claims every link (so no inner link
    /// is recorded again) and returns `true`.
    fn claim_write_chain(&mut self, e: &Expr) -> bool {
        let mut links: Vec<&Expr> = Vec::new();
        let mut cur = e;
        loop {
            links.push(cur);
            if let Some(m) = app_call(cur, &self.app, APP_WRITE) {
                for l in links {
                    self.claim(l);
                    if let Expr::MethodCall(mc) = l {
                        self.claim(mc);
                    }
                }
                self.claim(m);
                self.claim(&*m.receiver);
                if let Expr::Paren(p) = &*m.receiver {
                    self.claim(&*p.expr);
                }
                return true;
            }
            match chain_next(cur) {
                Some(n) => cur = n,
                None => return false,
            }
        }
    }

    /// W3: `app` passed to `callee` at position `i` of `call`.
    fn resolve_callee(&mut self, call: &ExprCall, i: usize) {
        let Expr::Path(fp) = &*call.func else {
            self.red(format!(
                "`{}` is passed to a call whose callee is not a path",
                self.app
            ));
            return;
        };
        let Some(name) = fp.path.get_ident().map(ToString::to_string) else {
            self.red(format!(
                "`{}` is passed to `{}`, a qualified path — only fns defined in runner.rs are resolved",
                self.app,
                short(&fp.path)
            ));
            return;
        };
        let Some(defs) = self.fns.get(&name) else {
            self.red(format!(
                "`{}` is passed to `{name}`, which is not a fn defined in runner.rs — the walk cannot see what it does",
                self.app
            ));
            return;
        };
        if defs.len() != 1 {
            self.red(format!(
                "`{name}` has {} definitions in runner.rs; the callee is ambiguous",
                defs.len()
            ));
            return;
        }
        if self.depth >= MAX_DEPTH {
            self.red(format!("callee depth {MAX_DEPTH} reached at `{name}`"));
            return;
        }
        let f = &defs[0];
        let Some(FnArg::Typed(param)) = f.sig.inputs.iter().nth(i) else {
            self.red(format!("`{name}` has no typed parameter #{i}"));
            return;
        };
        let Pat::Ident(pi) = &*param.pat else {
            self.red(format!("`{name}` parameter #{i} is not a plain identifier"));
            return;
        };
        if app_ref_type(&param.ty).is_none() {
            self.red(format!(
                "`{name}` receives `{}` as `{}`, not `&mut App` / `&App`",
                self.app,
                short(&*param.ty)
            ));
            return;
        }
        let mut inner = Walker::new(self.fns, pi.ident.to_string(), self.depth + 1);
        inner.visit_block(&f.block);
        for r in inner.out.reds {
            self.out.reds.push(format!("{name} → {r}"));
        }
        let site = key(call);
        if inner.out.sites.iter().any(|(k, _)| *k == Kind::Write) {
            self.out.sites.push((Kind::Write, site));
        } else if inner.out.sites.iter().any(|(k, _)| *k == Kind::Scheduler) {
            self.out.sites.push((Kind::Scheduler, site));
        } else {
            self.out.reads += 1 + inner.out.reads;
        }
    }

    /// The arguments of a call or method call: `app`, a write chain, or a `&World`.
    fn classify_args<'a>(
        &mut self,
        callee: &str,
        args: impl Iterator<Item = &'a Expr>,
        call: Option<&ExprCall>,
    ) {
        for (i, a) in args.enumerate() {
            if let Some(p) = as_ident_path(a, &self.app) {
                self.claim(p);
                self.claim(a);
                match call {
                    Some(c) => self.resolve_callee(c, i),
                    None => self.red(format!(
                        "`{}` is passed to the method `{callee}` — a method callee cannot be resolved",
                        self.app
                    )),
                }
                continue;
            }
            let inner = strip_ref(a);
            if self.world_value(inner).is_some() {
                self.claim_world_value(inner);
                self.red(format!(
                    "a &World escapes into `{callee}(..)` argument #{i} (`{}`) — the walk cannot see \
                     what the callee does with it, and `EcsMaster::send_event` writes through &self",
                    short(a)
                ));
                continue;
            }
            if self.claim_write_chain(inner) {
                self.out
                    .sites
                    .push((Kind::Write, format!("{callee}(#{i}: {})", key(inner))));
            }
        }
    }
}

impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_stmt(&mut self, s: &'ast Stmt) {
        let prev = std::mem::replace(&mut self.stmt, short(s));
        syn::visit::visit_stmt(self, s);
        self.stmt = prev;
    }

    fn visit_item(&mut self, i: &'ast Item) {
        // A nested item has its own scope; it may not name `app` at all.
        let names = [self.app.clone()];
        if tokens_name_any(i.to_token_stream(), &names) {
            self.red(format!("a nested item names `{}`", self.app));
        }
    }

    fn visit_local(&mut self, l: &'ast Local) {
        let mut bound = Vec::new();
        pat_idents(&l.pat, &mut bound);
        let init_is_world = l.init.as_ref().is_some_and(|i| {
            i.diverge.is_none() && app_call(&i.expr, &self.app, APP_READ).is_some()
        });
        for b in &bound {
            if *b == self.app || self.worlds.contains(b) {
                if init_is_world && self.worlds.contains(b) {
                    continue;
                }
                self.red(format!("`let` rebinds `{b}`, a tracked binding"));
            }
        }
        if init_is_world {
            match (&l.pat, bound.len()) {
                (Pat::Ident(pi), 1) if pi.subpat.is_none() && pi.by_ref.is_none() => {
                    let init = &l.init.as_ref().expect("invariant: checked above").expr;
                    self.claim_world_value(init);
                    if !self.worlds.contains(&pi.ident.to_string()) {
                        self.worlds.push(pi.ident.to_string());
                    }
                    self.out.reads += 1;
                }
                _ => self.red("`app.world()` bound by a non-identifier pattern".to_string()),
            }
        }
        syn::visit::visit_local(self, l);
    }

    fn visit_expr_closure(&mut self, c: &'ast ExprClosure) {
        let mut bound = Vec::new();
        for p in &c.inputs {
            pat_idents(p, &mut bound);
        }
        for b in &bound {
            if *b == self.app || self.worlds.contains(b) {
                self.red(format!(
                    "a closure parameter shadows `{b}`, a tracked binding"
                ));
            }
        }
        syn::visit::visit_expr_closure(self, c);
    }

    fn visit_expr_call(&mut self, c: &'ast ExprCall) {
        let callee = key(&*c.func);
        self.classify_args(&callee, c.args.iter(), Some(c));
        syn::visit::visit_expr_call(self, c);
    }

    fn visit_expr_method_call(&mut self, m: &'ast ExprMethodCall) {
        let method = m.method.to_string();
        let on_app = as_ident_path(&m.receiver, &self.app).is_some();
        if on_app {
            self.claim(&*m.receiver);
            if let Expr::Paren(p) = &*m.receiver {
                self.claim(&*p.expr);
            }
            if method == APP_READ {
                if !self.is_claimed(m) {
                    self.red(format!(
                        "`{}.world()` is used outside a READ method call or a `let` binding — a &World \
                         the walk cannot follow",
                        self.app
                    ));
                }
            } else if method == APP_WRITE {
                if !self.is_claimed(m) {
                    self.claim(m);
                    self.out.sites.push((Kind::Write, key(m)));
                }
            } else if method == APP_SCHEDULER {
                if !self.is_claimed(m) {
                    self.claim(m);
                    self.out.sites.push((Kind::Scheduler, key(m)));
                }
            } else {
                self.red(format!(
                    "`{}.{method}(..)` — a method on the App that the walk does not classify \
                     (read: {APP_READ}; write: {APP_WRITE}; scheduler: {APP_SCHEDULER})",
                    self.app
                ));
            }
        } else if let Some(w) = self.world_value(&m.receiver) {
            self.claim_world_value(w);
            if READ_METHODS.contains(&method.as_str()) {
                self.out.reads += 1;
            } else {
                self.red(format!(
                    "`.{method}(..)` on a &World is not in READ_METHODS {READ_METHODS:?}"
                ));
            }
        } else if !self.is_claimed(m) && self.claim_write_chain(&m.receiver) {
            // The outermost link of a chain rooted at `app.world_mut()`: one site, keyed by the
            // whole chain; `claim_write_chain` marked the inner links so none is recorded again.
            self.claim(m);
            self.out.sites.push((Kind::Write, key(m)));
        }
        self.classify_args(&format!(".{method}"), m.args.iter(), None);
        syn::visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_path(&mut self, p: &'ast ExprPath) {
        if p.qself.is_none() && !self.is_claimed(p) {
            if p.path.is_ident(self.app.as_str()) {
                self.red(format!(
                    "`{}` appears outside a method receiver or a call argument — an unclassified use \
                     of the App",
                    self.app
                ));
            } else if let Some(w) = self.worlds.iter().find(|w| p.path.is_ident(w.as_str())) {
                let w = w.clone();
                self.red(format!(
                    "the &World binding `{w}` is used outside a READ method call"
                ));
            }
        }
        syn::visit::visit_expr_path(self, p);
    }

    fn visit_expr(&mut self, e: &'ast Expr) {
        // `claim_world_value` / `claim_write_chain` claim the `Expr` wrapper as well as the inner
        // node; forward the claim to the inner node so the typed visitors above see it.
        if self.is_claimed(e) {
            match e {
                Expr::Path(p) => self.claim(p),
                Expr::MethodCall(m) => self.claim(m),
                _ => {}
            }
        }
        syn::visit::visit_expr(self, e);
    }

    fn visit_macro(&mut self, m: &'ast Macro) {
        let mut names = self.worlds.clone();
        names.push(self.app.clone());
        if !tokens_name_any(m.tokens.clone(), &names) {
            return;
        }
        match m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(exprs) => {
                for e in &exprs {
                    self.visit_expr(e);
                }
            }
            Err(_) => self.red(format!(
                "macro `{}!` names `{}` or a &World binding and does not parse as expressions",
                short(&m.path),
                self.app
            )),
        }
    }
}

/// Walks `frame_loop` with `fns` as the callee table.
fn walk_frame_loop(f: &ItemFn, fns: &BTreeMap<String, Vec<ItemFn>>) -> Walk {
    let mut w = Walker::new(fns, "app".to_string(), 0);
    w.visit_block(&f.block);
    w.out
}

// ═══════════════════════════════════════════════════════════════════════════
// Structural preconditions
// ═══════════════════════════════════════════════════════════════════════════

struct Tree {
    frame_loop: ItemFn,
    fns: BTreeMap<String, Vec<ItemFn>>,
}

fn load() -> Tree {
    let runner = parse(&manifest_dir().join("src/runner.rs"));
    let loops: Vec<&ItemFn> = runner
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Fn(f) if f.sig.ident == "frame_loop" => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(
        loops.len(),
        1,
        "G-LOOP ANTI-VACUITY: expected exactly one top-level `fn frame_loop` in runner.rs, found {}",
        loops.len()
    );
    let frame_loop = loops[0].clone();
    let fns = fn_table(&runner);
    Tree { frame_loop, fns }
}

/// `frame_loop(app: &mut App, host: &mut WindowHost, ctx: &'static VulkanContext)` — the World is
/// reachable only through `app`. A changed signature re-opens that argument.
fn assert_frame_loop_signature(f: &ItemFn) {
    let sig: Vec<String> = f.sig.inputs.iter().map(key).collect();
    assert_eq!(
        sig,
        [
            "app:&mut App",
            "host:&mut WindowHost",
            "ctx:&'static VulkanContext"
        ],
        "G-LOOP: frame_loop's signature changed — re-establish that the World is reachable only \
         through `app` before updating this assertion"
    );
}

/// `WindowHost`'s direct fields name no `App` / `EcsMaster`.
fn assert_window_host_holds_no_world() {
    let host = parse(&manifest_dir().join("src/host.rs"));
    let mut found = false;
    for item in &host.items {
        if let Item::Struct(s) = item
            && s.ident == "WindowHost"
        {
            found = true;
            for field in &s.fields {
                let ty = key(&field.ty);
                let names = [
                    "App".to_string(),
                    "EcsMaster".to_string(),
                    "World".to_string(),
                ];
                assert!(
                    !tokens_name_any(field.ty.to_token_stream(), &names),
                    "G-LOOP: WindowHost field `{}: {ty}` names the World — frame_loop can reach it \
                     outside `app`, and this walk does not follow it",
                    field
                        .ident
                        .as_ref()
                        .map_or_else(String::new, ToString::to_string)
                );
            }
        }
    }
    assert!(
        found,
        "G-LOOP: `struct WindowHost` not found in src/host.rs"
    );
}

/// The `&self` / `&mut self` shape of every method of every `impl <ty>` in `files`.
fn impl_methods(files: &[(PathBuf, syn::File)], ty: &str) -> Vec<(String, bool, String, PathBuf)> {
    let mut out = Vec::new();
    for (path, file) in files {
        for item in &file.items {
            let Item::Impl(imp) = item else { continue };
            if imp.trait_.is_some() {
                continue;
            }
            let Type::Path(tp) = &*imp.self_ty else {
                continue;
            };
            if tp.path.segments.last().is_none_or(|s| s.ident != ty) {
                continue;
            }
            for ii in &imp.items {
                let ImplItem::Fn(f) = ii else { continue };
                let Some(FnArg::Receiver(r)) = f.sig.inputs.first() else {
                    continue;
                };
                if r.reference.is_none() {
                    continue;
                }
                let ret = match &f.sig.output {
                    syn::ReturnType::Default => String::new(),
                    syn::ReturnType::Type(_, t) => key(&**t),
                };
                out.push((
                    f.sig.ident.to_string(),
                    r.mutability.is_some(),
                    ret,
                    path.clone(),
                ));
            }
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

/// [`READ_METHODS`] are `&self` on `EcsMaster` and hand out no `&mut` / `Mut<`.
#[test]
fn read_methods_are_shared_and_non_mut() {
    let dir = manifest_dir().join("../boyko_ecs/src/ecs/core/ecs_master");
    let files = parse_dir(&dir);
    let methods = impl_methods(&files, "EcsMaster");
    assert!(
        methods.len() > 50,
        "ANTI-VACUITY: only {} `impl EcsMaster` methods parsed under {}",
        methods.len(),
        dir.display()
    );
    for m in READ_METHODS {
        let defs: Vec<_> = methods.iter().filter(|(n, ..)| n == m).collect();
        assert!(
            !defs.is_empty(),
            "READ method `{m}` is not an `impl EcsMaster` method"
        );
        for (n, is_mut, ret, path) in defs {
            assert!(
                !is_mut,
                "READ method `{n}` takes `&mut self` ({})",
                path.display()
            );
            assert!(
                !ret.contains("&mut") && !ret.contains("Mut<"),
                "READ method `{n}` returns `{ret}` ({})",
                path.display()
            );
        }
    }
}

/// The three App methods the walk classifies have the receivers the classification assumes, and
/// the `&mut self` set of EVERY `impl App` block is printed (both files — `add_plugins` is in
/// `plugins.rs`).
#[test]
fn app_methods_are_classified_by_receiver() {
    let dir = manifest_dir().join("../boyko_ecs/src/ecs/core/app");
    let files = parse_dir(&dir);
    let methods = impl_methods(&files, "App");
    let files_with_impl: std::collections::BTreeSet<&PathBuf> =
        methods.iter().map(|m| &m.3).collect();
    assert!(
        files_with_impl.len() >= 2,
        "ANTI-VACUITY: `impl App` methods found in {} file(s) under {}; `app.rs` and `plugins.rs` \
         both define them",
        files_with_impl.len(),
        dir.display()
    );
    let find = |n: &str| methods.iter().find(|m| m.0 == n);
    let world = find(APP_READ).expect("App::world");
    assert!(
        !world.1 && world.2 == "&EcsMaster",
        "App::world must be `&self -> &EcsMaster`, is {world:?}"
    );
    let world_mut = find(APP_WRITE).expect("App::world_mut");
    assert!(world_mut.1, "App::world_mut must take `&mut self`");
    let sched = find(APP_SCHEDULER).expect("App::update_with_delta");
    assert!(sched.1, "App::update_with_delta must take `&mut self`");
    let mut muts: Vec<&str> = methods
        .iter()
        .filter(|m| m.1)
        .map(|m| m.0.as_str())
        .collect();
    muts.sort_unstable();
    muts.dedup();
    assert!(
        muts.contains(&"add_plugins"),
        "ANTI-VACUITY: plugins.rs's `add_plugins` was not read"
    );
    println!(
        "impl App `&mut self` methods ({}), every one RED on `app` except `{APP_WRITE}` (site) and \
         `{APP_SCHEDULER}` (scheduler): {}",
        muts.len(),
        muts.join(", ")
    );
}

/// The gate: the detected multiset equals [`ALLOW`] exactly, and the pins equal [`ALLOW`].
#[test]
fn g_loop_runner_world_writes_match_the_allowlist() {
    let tree = load();
    assert_frame_loop_signature(&tree.frame_loop);
    assert_window_host_holds_no_world();
    let walk = walk_frame_loop(&tree.frame_loop, &tree.fns);

    let mut violations: Vec<String> = Vec::new();
    for r in &walk.reds {
        violations.push(format!("UNCLASSIFIED: {r}"));
    }

    // Anti-vacuity: the one scheduled step was found, and the walk saw reads.
    let schedulers = walk
        .sites
        .iter()
        .filter(|(k, _)| *k == Kind::Scheduler)
        .count();
    if schedulers != 1 {
        violations.push(format!(
            "ANTI-VACUITY: expected exactly one SCHEDULER site (F4), found {schedulers}"
        ));
    }
    if walk.reads == 0 {
        violations.push("ANTI-VACUITY: the walk classified no &World read".to_string());
    }

    // Detected vs allowed, as multisets over (kind, key).
    let mut detected: BTreeMap<(Kind, String), u32> = BTreeMap::new();
    for (k, s) in &walk.sites {
        *detected.entry((*k, s.clone())).or_default() += 1;
    }
    let mut allowed: BTreeMap<(Kind, String), u32> = BTreeMap::new();
    for (_, class, k, n) in ALLOW {
        *allowed.entry((class.kind(), (*k).to_string())).or_default() += n;
    }
    for ((kind, k), n) in &detected {
        let a = allowed.get(&(*kind, k.clone())).copied().unwrap_or(0);
        if a != *n {
            violations.push(format!(
                "{} {kind:?} site ×{n} (allowlisted ×{a}): {k}",
                if a < *n { "UNLISTED" } else { "COUNT" }
            ));
        }
    }
    for ((kind, k), a) in &allowed {
        if !detected.contains_key(&(*kind, k.clone())) {
            violations.push(format!(
                "VANISHED {kind:?} site (allowlisted ×{a}, detected ×0): {k} — lower its row and the pins \
                 in the same commit"
            ));
        }
    }

    // The pins, derived from the allowlist's COUNTED rows.
    let mut steps: Vec<&str> = ALLOW
        .iter()
        .filter(|r| r.1 == Class::Counted)
        .map(|r| r.0)
        .collect();
    steps.sort_unstable();
    steps.dedup();
    let sites: u32 = ALLOW
        .iter()
        .filter(|r| r.1 == Class::Counted)
        .map(|r| r.3)
        .sum();
    if steps.len() != G_LOOP_STEPS || sites as usize != G_LOOP_SITES {
        violations.push(format!(
            "PINS: ALLOW's COUNTED rows give {} steps / {sites} sites, pinned {G_LOOP_STEPS} / \
             {G_LOOP_SITES} — the pins and the allowlist move together, and only down",
            steps.len()
        ));
    }

    // Report.
    for (step, class, k, n) in ALLOW {
        let seen = detected
            .get(&(class.kind(), (*k).to_string()))
            .copied()
            .unwrap_or(0);
        println!(
            "G-LOOP {step:<4} {:<9} ×{n} (seen ×{seen})  {k}",
            format!("{class:?}")
        );
    }
    let exempt: u32 = ALLOW
        .iter()
        .filter(|r| r.1 == Class::Exempt)
        .map(|r| r.3)
        .sum();
    let sched: u32 = ALLOW
        .iter()
        .filter(|r| r.1 == Class::Scheduler)
        .map(|r| r.3)
        .sum();
    println!(
        "G-LOOP: {} steps / {sites} sites outside a system (design row 120 says 7: +F9 +F14 \
         undercounted); exempt F2 {exempt}; scheduler F4 {sched}; &World reads classified {}",
        steps.len(),
        walk.reads
    );

    if !violations.is_empty() {
        println!("\nDetected multiset (paste-ready keys; assign each its host step):");
        for ((kind, k), n) in &detected {
            println!("    ({kind:?}) ×{n}  {k:?}");
        }
        for v in &violations {
            println!("G-LOOP VIOLATION: {v}");
        }
        panic!(
            "G-LOOP: {} violation(s) — see the lines above. A new site is not allowlisted by \
             adding a row: it is moved into a system, or the plan is changed.",
            violations.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Red controls — permanent
// ═══════════════════════════════════════════════════════════════════════════

/// The top-level `loop { .. }` of `frame_loop`.
fn loop_body(f: &mut ItemFn) -> &mut Block {
    for s in &mut f.block.stmts {
        if let Stmt::Expr(Expr::Loop(l), _) = s {
            return &mut l.body;
        }
    }
    panic!("G-LOOP: `frame_loop` has no top-level `loop`");
}

/// Re-walks `frame_loop` with `stmts` inserted at the top of its loop and `extra` fns added.
fn mutated(tree: &Tree, stmts: &str, extra: &[&str]) -> Walk {
    let mut f = tree.frame_loop.clone();
    let block: Block = syn::parse_str(&format!("{{ {stmts} }}"))
        .unwrap_or_else(|e| panic!("control statements do not parse: {e}: {stmts}"));
    let body = loop_body(&mut f);
    for (i, s) in block.stmts.into_iter().enumerate() {
        body.stmts.insert(i, s);
    }
    let mut fns = tree.fns.clone();
    for src in extra {
        let item: ItemFn =
            syn::parse_str(src).unwrap_or_else(|e| panic!("control fn does not parse: {e}"));
        fns.entry(item.sig.ident.to_string())
            .or_default()
            .push(item);
    }
    walk_frame_loop(&f, &fns)
}

fn site_multiset(w: &Walk) -> Vec<(Kind, String)> {
    let mut v = w.sites.clone();
    v.sort();
    v
}

#[test]
fn walk_fails_closed_on_every_unclassified_shape() {
    let tree = load();
    let base = walk_frame_loop(&tree.frame_loop, &tree.fns);
    let base_sites = site_multiset(&base);
    const WRITE: &str = "app.world_mut().insert_resource(WindowInfo { width: 0, height: 0 });";
    const POKE: &str = "fn poke(app: &mut App) { app.world_mut().insert_resource(WindowInfo { width: 0, height: 0 }); }";
    const SEND: &str = "fn helper_send(w: &EcsMaster) { let _ = w.send_event(0, Ping); }";

    // Each control: (label, statements, extra fns, a substring the RED must carry).
    let red_controls: &[(&str, &str, &[&str], &str)] = &[
        ("(i) a new world_mut write", WRITE, &[], "UNLISTED"),
        (
            "(ii) the same write inside debug_assert!",
            "debug_assert!({ app.world_mut().insert_resource(WindowInfo { width: 0, height: 0 }); true });",
            &[],
            "UNLISTED",
        ),
        (
            "(iii) a runner.rs fn taking &mut App",
            "poke(app);",
            &[POKE],
            "UNLISTED",
        ),
        (
            "(iii-b) a callee not defined in runner.rs",
            "poke_elsewhere(app);",
            &[],
            "not a fn defined in runner.rs",
        ),
        (
            "(v) app.add_plugins",
            "app.add_plugins(boyko_app::FlyCameraPlugin);",
            &[],
            "does not classify",
        ),
        (
            "(vi) a &World passed to a helper that sends an event",
            "helper_send(app.world());",
            &[SEND],
            "escapes",
        ),
        (
            "(vii) the same through a let binding",
            "let w2 = app.world(); helper_send(w2);",
            &[SEND],
            "escapes",
        ),
        (
            "(viii) (*app).world_mut()",
            "(*app).world_mut().insert_resource(WindowInfo { width: 0, height: 0 });",
            &[],
            "outside a method receiver",
        ),
        (
            "(ix) let a = &mut *app",
            "let a = &mut *app; a.world_mut().insert_resource(WindowInfo { width: 0, height: 0 });",
            &[],
            "outside a method receiver",
        ),
        (
            "(x) send_event on a &World binding",
            "let w3 = app.world(); let _ = w3.send_event(0, Ping);",
            &[],
            "not in READ_METHODS",
        ),
        (
            "(xi) a let that shadows app",
            "let app = 0u8;",
            &[],
            "rebinds `app`",
        ),
        (
            "(xii) an unparsable macro that names app",
            "my_macro!(app => world_mut);",
            &[],
            "does not parse",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (label, stmts, extra, needle) in red_controls {
        let w = mutated(&tree, stmts, extra);
        let unlisted = site_multiset(&w) != base_sites;
        let reds = w.reds.join(" | ");
        let red = unlisted || !w.reds.is_empty();
        let carries = if *needle == "UNLISTED" {
            unlisted
        } else {
            reds.contains(needle)
        };
        println!(
            "control {label}: {}{}",
            if red { "RED" } else { "GREEN" },
            if unlisted {
                " (site multiset changed)"
            } else {
                ""
            }
        );
        if !reds.is_empty() {
            println!("    reds: {reds}");
        }
        if !red || !carries {
            failures.push(format!(
                "{label}: expected RED carrying `{needle}`, got red={red}, reds=[{reds}]"
            ));
        }
    }

    // (iv) GREEN: one more classified read, nothing else moves.
    let w = mutated(&tree, "let _ = app.world().resource::<WindowInfo>();", &[]);
    println!(
        "control (iv) an added &World read: reads {} -> {}, reds {}",
        base.reads,
        w.reads,
        w.reds.len()
    );
    if site_multiset(&w) != base_sites || !w.reds.is_empty() || w.reads != base.reads + 1 {
        failures.push(format!(
            "(iv) GREEN control: expected the same sites, no reds and reads+1; got reds {:?}, reads {} -> {}",
            w.reds, base.reads, w.reads
        ));
    }
    assert!(
        failures.is_empty(),
        "G-LOOP red controls failed:\n{}",
        failures.join("\n")
    );
}
