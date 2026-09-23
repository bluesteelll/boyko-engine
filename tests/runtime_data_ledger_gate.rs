//! **UG-02: the runtime data ledger gate** (unified plan 03 UG-02 = allocator G1 + engine G-FORM +
//! physics G-form; landed at rung B1 with ledger rev 5).
//!
//! Every heap site in the non-test code of a scanned crate must be a row of
//! `docs/memory/runtime-data-ledger.tsv`, and every row must be a site: the comparison is an
//! exact multiset over `(file, kind|item_path|container)`, line-free, so a line shift is not a
//! diff. The scanner walks the syntax tree (`syn`), not the text: a text scanner misses tuple
//! variants, aliases, `cfg(not(test))`, hand-wrapped generics and every head but `Vec<`
//! (allocator design P6, W4). The shapes, the comparison set, the pins and the controls are
//! specified in the rev-5 part of `docs/memory/RUNTIME-DATA-LEDGER.md`.
//!
//! # Pins
//!
//! `docs/memory/ledger/ug02-pins.tsv` is append-only. Its last line must equal the counts over the
//! ledger; line to line, every count only falls, except on a `merge-raise` line with a reason (a
//! merge that adds rows re-derives them in the same commit, 03 §2). The first line is anchored
//! here, in [`GENESIS`], and every later line carries the FNV-1a hash of the one before, so the
//! history cannot be rewritten in place without a gate-code edit (critique W4 (a)).
//!
//! # Fixtures
//!
//! Every fixture is an inline source written under `CARGO_TARGET_TMPDIR` at test time and run
//! through the same pipeline. There are no fixture `.rs` files in the repository, so no other
//! source census reads them. Each fixture asserts the EXACT site multiset (two-sided).

#[path = "ledger_scan_support/mod.rs"]
mod scan;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The first line of `docs/memory/ledger/ug02-pins.tsv`: rung B1's pins (ledger rev 5).
const GENESIS: &str = "B1\tpin\t1660\t2825\t36\t1155\t5\t5\tecs-storage=203;ecs-schedule=174;ecs-services=110;pool-utils-log=176;physics-scene-math=104;render=161;rhi=111;ui-input=94;app-demo=297;codec-tools=527;macros-aether=8;ui-lane=318;reflect-lane=7\t-\tledger rev 5: the three census trees re-derived on the integ/unified trunk c1e9f1db by the syn scanner (unified plan 02 B1; 03 UG-02)";

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn no_lines(_: proc_macro2::Span) -> Option<u32> {
    None
}

fn real_report() -> &'static scan::GateReport {
    static REPORT: OnceLock<scan::GateReport> = OnceLock::new();
    REPORT.get_or_init(|| {
        scan::run_gate(&scan::GateConfig {
            data_root: repo_root(),
            scan_root: repo_root(),
            genesis: Some(GENESIS),
            baseline: scan::BASELINE_SCANNED,
            line_of: &no_lines,
        })
    })
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The gate on the real tree
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn ledger_matches_the_trunk() {
    let rep = real_report();
    println!("{}", rep.render());
    // Anti-vacuity: a gate that scanned nothing and compared nothing is not green.
    assert!(rep.files > 0, "no file was parsed");
    assert!(rep.sites > 0, "no site was scanned");
    assert!(rep.compared > 0, "the comparison set is empty");
    assert!(
        rep.failures.is_empty(),
        "UG-02 RED: {} failures ({} extra, {} missing)",
        rep.failures.len(),
        rep.extra,
        rep.missing
    );
}

#[test]
fn pins_equal_the_ledger_and_only_decrease() {
    let pins = real_report().with_tag("PIN");
    assert!(pins.is_empty(), "pin failures:\n{}", join(&pins));
}

#[test]
fn every_workspace_member_is_declared() {
    let members = real_report().with_tag("MEMBER");
    assert!(members.is_empty(), "member failures:\n{}", join(&members));
}

#[test]
fn empty_scan_root_is_red_on_the_real_floors() {
    let empty = tmp("empty_scan_root");
    let rep = scan::run_gate(&scan::GateConfig {
        data_root: repo_root(),
        scan_root: &empty,
        genesis: Some(GENESIS),
        baseline: scan::BASELINE_SCANNED,
        line_of: &no_lines,
    });
    show_red("empty_scan_root_is_red_on_the_real_floors", &rep);
    let floors = rep.with_tag("FLOOR");
    assert_eq!(
        floors.len(),
        scan::GROUPS.len(),
        "an empty scan root must be RED on every group's floor, got:\n{}",
        rep.render()
    );
}

fn join(v: &[&String]) -> String {
    v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Fixture plumbing
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn tmp(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("ug02")
        .join(name);
    if root.exists() {
        fs::remove_dir_all(&root).expect("invariant: the fixture dir is ours to remove");
    }
    fs::create_dir_all(&root).expect("invariant: CARGO_TARGET_TMPDIR is writable");
    root
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(
        p.parent()
            .expect("invariant: a relative file path has a parent"),
    )
    .expect("invariant: CARGO_TARGET_TMPDIR is writable");
    fs::write(p, text).expect("invariant: CARGO_TARGET_TMPDIR is writable");
}

/// Scans `files` as crate `crates/c` (full) plus `extra` crates `(path, mode)`, with the baseline
/// vocabulary plus the repository's `ug02-vocab.tsv` additions.
fn scan_fixture(name: &str, files: &[(&str, &str)], extra: &[(&str, &str)]) -> scan::ScanOut {
    let root = tmp(name);
    let mut decls = String::from("kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\n");
    for (p, m) in extra {
        decls.push_str(&format!("crate\t{p}\t{m}\tfixture\n"));
    }
    for (rel, text) in files {
        write(&root, rel, text);
    }
    let decls = scan::parse_decls(&decls).expect("invariant: fixture decls parse");
    let vocab_text = fs::read_to_string(repo_root().join(scan::VOCAB_TSV))
        .expect("invariant: the repository vocabulary file exists");
    let vocab = scan::Vocab::with_additions(&vocab_text).expect("invariant: vocabulary parses");
    scan::scan(&root, &decls, &vocab, &no_lines)
}

/// `file|key` for every site, sorted.
fn keys(out: &scan::ScanOut) -> Vec<String> {
    let mut v: Vec<String> = out
        .sites
        .iter()
        .map(|s| format!("{}|{}", s.file, s.key()))
        .collect();
    v.sort();
    v
}

fn expect_sites(out: &scan::ScanOut, file: &str, want: &[&str]) {
    let mut want: Vec<String> = want.iter().map(|k| format!("{file}|{k}")).collect();
    want.sort();
    assert!(
        out.failures.is_empty(),
        "unexpected scan failures: {:?}",
        out.failures
    );
    assert_eq!(keys(out), want, "site multiset differs");
}

const LIB: &str = "crates/c/src/lib.rs";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// P6's seven hole fixtures (allocator design P6: W4's text-scanner holes and the shapes they hide)
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn f1_enum_payloads_and_union_fields() {
    let src = "pub enum E { A(Vec<u8>), B { s: String } }\n\
               pub union U { b: std::mem::ManuallyDrop<Box<u8>> }\n";
    let out = scan_fixture("f1", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &[
            "field|E::A.0|Vec",
            "field|E::B.s|String",
            "field|U::b|Box<T>",
        ],
    );
}

#[test]
fn f2_aliases_resolve_and_an_ambiguous_alias_is_red() {
    let src = "type Buf = Vec<u8>;\ntype L<T> = Vec<T>;\ntype C = Buf;\n\
               pub struct S { a: Buf, b: L<u32>, c: C }\n";
    let out = scan_fixture("f2", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &["field|S::a|Vec", "field|S::b|Vec", "field|S::c|Vec"],
    );

    let amb = "mod m1 { pub type Amb = Vec<u8>; }\nmod m2 { pub type Amb = String; }\n\
               pub struct T { a: Amb }\n";
    let out = scan_fixture("f2_ambiguous", &[(LIB, amb)], &[]);
    assert!(
        out.failures
            .iter()
            .any(|f| f.starts_with("ALIAS-AMBIGUOUS")),
        "an alias defined twice with different heads must be RED, got {:?}",
        out.failures
    );
}

#[test]
fn f3_cfg_not_test_is_counted_and_test_twins_are_not() {
    let src = "#[cfg(not(test))] pub struct A { v: Vec<u8> }\n\
               #[cfg(test)] pub struct B { v: Vec<u8> }\n\
               #[cfg(all(test, windows))] pub struct C { v: Vec<u8> }\n\
               #[cfg(miri)] pub struct D { v: Vec<u8> }\n\
               #[cfg(loom)] pub struct E { v: Vec<u8> }\n\
               #[cfg(any(test, feature = \"x\"))] pub struct F { v: Vec<u8> }\n";
    let out = scan_fixture("f3", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["field|A::v|Vec", "field|F::v|Vec"]);
}

#[test]
fn f4_a_head_after_the_first_comma_of_a_wrapped_type() {
    // The head is the SECOND argument of a non-head type, over four lines (critique W1): a
    // scanner that reads only the first generic argument finds nothing here.
    let src = "pub struct S {\n    m: Result<\n        (),\n        Vec<u8>,\n    >,\n}\n";
    let out = scan_fixture("f4", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["field|S::m|Vec"]);
}

#[test]
fn f5_boxed_trait_objects() {
    let src = "pub struct S { f: Box<dyn Fn() + Send>, a: Box<dyn std::any::Any> }\n";
    let out = scan_fixture("f5", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["field|S::a|Box<dyn>", "field|S::f|Box<dyn>"]);
}

#[test]
fn f6_boxed_and_shared_slices() {
    let src = "pub struct S<T> { b: Box<[u8]>, r: std::rc::Rc<[T]>, a: std::sync::Arc<str> }\n";
    let out = scan_fixture("f6", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &["field|S::a|Arc", "field|S::b|Box<[T]>", "field|S::r|Rc"],
    );
}

#[test]
fn f7_path_forms_collapse_to_the_head() {
    let src = "use std::collections::HashMap as Map;\n\
               pub struct S<T> { a: std::vec::Vec<T>, b: alloc::vec::Vec<T>, c: ::std::string::String, m: Map<u32, u32> }\n";
    let out = scan_fixture("f7", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &[
            "field|S::a|Vec",
            "field|S::b|Vec",
            "field|S::c|String",
            "field|S::m|HashMap",
        ],
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Shape fixtures (P7's lint-blind shapes that P7 hands to G1, and UG-02's own kinds)
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn k1_constructor_calls() {
    let src = "pub fn f() { let v = Vec::<u8>::new(); let w = Vec::<u8>::with_capacity(4); drop((v, w)); }\n";
    let out = scan_fixture("k1", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|Vec", "local|f|Vec"]);
}

#[test]
fn k2_collect_with_turbofish() {
    let src = "pub fn f(it: std::ops::Range<u32>) -> usize { it.collect::<Vec<_>>().len() }\n";
    let out = scan_fixture("k2", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|Vec"]);
}

#[test]
fn k3_vec_macros() {
    let src = "pub fn f() { let _ = (vec![0u8; 4], vec![1u8]); }\n";
    let out = scan_fixture("k3", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|Vec", "local|f|Vec"]);
}

#[test]
fn k4_inferred_collect() {
    let src = "pub fn f(it: std::ops::Range<u32>) { let v = it.collect(); g(v); }\n";
    let out = scan_fixture("k4", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|inferred"]);
}

#[test]
fn k5_strings() {
    let src = "pub fn f(x: u32) { let a = format!(\"{x}\"); let b = x.to_string(); let c = String::from(\"a\"); drop((a, b, c)); }\n";
    let out = scan_fixture("k5", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &["local|f|String", "local|f|String", "local|f|String"],
    );
}

#[test]
fn k6_macro_bodies_invocations_and_quote_templates() {
    let src = "macro_rules! m { () => { Box::new(1) } }\n\
               pub fn f(ok: bool) { my_macro!(Box::new(1)); assert!(ok, \"{}\", format!(\"x\")); }\n";
    let mac = "pub fn t() -> proc_macro2::TokenStream { quote::quote! { let v = Vec::new(); } }\n";
    let out = scan_fixture(
        "k6",
        &[(LIB, src), ("crates/mac/src/lib.rs", mac)],
        &[("crates/mac", "emitted")],
    );
    assert!(out.failures.is_empty(), "{:?}", out.failures);
    let mut want = vec![
        format!("{LIB}|macro|macro_rules!m|Box<T>"),
        format!("{LIB}|macro|my_macro!|Box<T>"),
        format!("{LIB}|local|f|String"),
        "crates/mac/src/lib.rs|emitted|crates/mac/src/lib.rs|Vec".to_owned(),
    ];
    want.sort();
    assert_eq!(keys(&out), want);
}

#[test]
fn k7_statics_thread_locals_and_their_initialisers() {
    // Critique O3: a static's initialiser is absorbed (one declaration, one site), a closure
    // initialiser included; a `const` is not a site and its initialiser is not walked.
    let src = "use std::cell::{Cell, RefCell};\nuse std::collections::HashMap;\n\
               use std::sync::{LazyLock, Mutex, OnceLock};\n\
               pub static S: Mutex<Vec<u8>> = Mutex::new(Vec::new());\n\
               pub static O: OnceLock<HashMap<u32, u32>> = OnceLock::new();\n\
               pub static L: LazyLock<HashMap<u32, u32>> = LazyLock::new(|| HashMap::new());\n\
               pub const E: Vec<u8> = Vec::new();\n\
               thread_local! {\n    static A: Cell<u32> = const { Cell::new(0) };\n    \
               static B: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(8));\n}\n";
    let out = scan_fixture("k7", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &[
            "static|static S|Vec",
            "static|static O|HashMap",
            "static|static L|HashMap",
            "static|tls A|tls-key",
            "static|tls B|Vec",
        ],
    );
}

#[test]
fn k8_params_and_returns() {
    let src = "pub fn f(v: Vec<u8>, out: &mut Vec<u8>, r: &Vec<u8>, s: &[u8], t: &str) -> Result<(), String> { drop((v, out, r, s, t)); Ok(()) }\n\
               pub trait T { fn g(&self) -> Vec<u8>; }\n";
    let out = scan_fixture("k8", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &["param|f|Vec", "param|f|Vec", "return|f|String"],
    );
}

#[test]
fn k9_a_typed_let_absorbs_its_constructor() {
    let src = "pub fn f() { let v: Vec<u8> = Vec::with_capacity(4); drop(v); }\n";
    let out = scan_fixture("k9", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|Vec"]);
}

#[test]
fn k10_raw_allocation() {
    let src = "pub fn f(layout: std::alloc::Layout) -> *mut u8 { unsafe { let p = std::alloc::alloc(layout); p } }\n";
    let out = scan_fixture("k10", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["local|f|alloc::alloc"]);
}

#[test]
fn k11_std_calls_sorts_spawns_and_pinned_clones() {
    // Critique W6: forms with a call site are syntactic, and `.clone()` counts when the syntax
    // pins the receiver's type (a field, a param, a typed or constructed local); an `Arc` clone
    // is a refcount increment, not an allocation.
    let src = "use std::process::Command;\n\
               pub struct H { names: Vec<String>, shared: std::sync::Arc<u8> }\n\
               impl H {\n    pub fn f(&mut self, mut v: Vec<u32>, p: &std::path::Path) {\n        \
               v.sort_by_key(|x| *x);\n        let _ = std::fs::File::open(p);\n        \
               let _ = std::fs::read(p);\n        let _ = Command::new(\"x\");\n        \
               let _ = std::thread::spawn(|| {});\n        \
               let b = std::thread::Builder::new().stack_size(1);\n        \
               let _ = b.spawn(|| {});\n        let n = self.names.clone();\n        \
               let s = self.shared.clone();\n        let w = v.clone();\n        drop((n, s, w));\n    }\n}\n";
    let out = scan_fixture("k11", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &[
            "field|H::names|Vec",
            "field|H::shared|Arc",
            "param|H::f|Vec",
            "local|H::f|sort-scratch",
            "local|H::f|File::open",
            "local|H::f|fs::read",
            "local|H::f|Command",
            "local|H::f|thread::spawn",
            "local|H::f|thread::spawn",
            "local|H::f|Vec",
            "local|H::f|Vec",
        ],
    );
}

#[test]
fn k12_a_constructor_named_as_a_value() {
    let src = "pub fn f(v: Vec<&str>) -> Vec<std::path::PathBuf> { v.into_iter().map(std::path::PathBuf::from).collect() }\n";
    let out = scan_fixture("k12", &[(LIB, src)], &[]);
    expect_sites(
        &out,
        LIB,
        &[
            "param|f|Vec",
            "return|f|Vec",
            "local|f|PathBuf",
            "local|f|inferred",
        ],
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Negatives
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn n1_test_code_is_not_scanned() {
    let src = "#[cfg(test)] mod tests { fn t() { let v = Vec::<u8>::new(); drop(v); } }\n\
               #[test] fn free() { let v = Vec::<u8>::new(); drop(v); }\n";
    let out = scan_fixture("n1", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &[]);
}

#[test]
fn n2_kernel_storage_is_transparent() {
    let src = "pub struct S { a: ScratchColumn<u32>, b: ScratchColumn<Vec<u8>> }\n";
    let out = scan_fixture("n2", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &["field|S::b|Vec"]);
}

#[test]
fn n3_mentions_borrows_markers_and_consts_are_not_sites() {
    let src = "/// A `Vec<u8>` in a doc comment.\n// Vec<u8> in a line comment\n\
               pub struct S<'a, T> { p: std::marker::PhantomData<Vec<T>>, r: &'a Vec<T> }\n\
               pub const E: Vec<u8> = Vec::new();\n\
               pub fn s() -> &'static str { \"Vec<u8>::new() String::from\" }\n\
               pub fn g() -> impl Iterator<Item = String> { std::iter::empty() }\n";
    let out = scan_fixture("n3", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &[]);
}

#[test]
fn n4_cfg_test_module_files_are_excluded_and_path_modules_are_followed() {
    // Critique W1: the followed `#[path]` file holds a site, so a walk that stops following
    // `#[path]` loses it; the cfg(test) file holds one too, so a walk that stops excluding
    // cfg(test) modules gains it.
    let lib = "#[cfg(test)] mod t;\n#[path = \"x_impl.rs\"] mod x;\n";
    let out = scan_fixture(
        "n4",
        &[
            (LIB, lib),
            ("crates/c/src/t.rs", "pub struct T { v: Vec<u8> }\n"),
            ("crates/c/src/x_impl.rs", "pub struct X { v: Vec<u8> }\n"),
        ],
        &[],
    );
    expect_sites(&out, "crates/c/src/x_impl.rs", &["field|X::v|Vec"]);
    assert!(
        out.excluded.contains("crates/c/src/t.rs"),
        "the cfg(test) file must be excluded"
    );
}

#[test]
fn n5_heads_in_non_declaration_positions() {
    // Critique O5.
    let src = "pub trait Foo {}\nimpl Foo for Vec<u8> {}\n\
               pub fn f<T>() -> usize where T: Into<String> {\n    \
               std::mem::size_of::<Vec<T>>() + usize::from(std::any::TypeId::of::<String>() == std::any::TypeId::of::<u8>())\n}\n";
    let out = scan_fixture("n5", &[(LIB, src)], &[]);
    expect_sites(&out, LIB, &[]);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The physics slice and the vocabulary sweep
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn p1_physics_slice() {
    // Critique O1: a fn-item argument and a `use … as` rename still count; `std::thread::scope`
    // (path or imported module) does not; test code does not.
    let src = "use boyko_threadpool::try_with_active_pool as twap;\nuse std::thread;\n\
               pub fn run(pool: &Pool) {\n    pool.scope(|s| { let _ = s; });\n    pool.scope(body);\n    \
               try_with_active_pool(|p| p.n());\n    twap(|p| p.n());\n    \
               std::thread::scope(|s| { let _ = s; });\n    thread::scope(|s| { let _ = s; });\n}\n\
               #[cfg(test)] mod tests { fn t(pool: &Pool) { pool.scope(|s| {}); super::try_with_active_pool(|p| p.n()); } }\n";
    let file = "crates/boyko_physics/src/lib.rs";
    let out = scan_fixture("p1", &[(file, src)], &[("crates/boyko_physics", "full")]);
    let scope = out.slice.iter().filter(|h| h.which == "scope").count();
    let active = out
        .slice
        .iter()
        .filter(|h| h.which == "try_with_active_pool")
        .count();
    assert_eq!(
        (scope, active),
        (2, 2),
        "slice hits: {:?}",
        out.slice.iter().map(|h| &h.text).collect::<Vec<_>>()
    );
}

#[test]
fn o6_every_head_is_found_as_a_field() {
    // Critique O6: one synthetic field per head, each must give exactly one site with that
    // container (closes the `Vec<`-only hole for every head, not only the ones F1-F7 use).
    let vocab_text = fs::read_to_string(repo_root().join(scan::VOCAB_TSV))
        .expect("invariant: the repository vocabulary file exists");
    let vocab = scan::Vocab::with_additions(&vocab_text).expect("invariant: vocabulary parses");
    let mut src = String::from("pub struct S {\n");
    let mut want = Vec::new();
    for (i, head) in vocab.heads.keys().enumerate() {
        src.push_str(&format!("    f{i}: {head}<u8>,\n"));
        let c = if head == "Box" {
            "Box<T>".to_owned()
        } else {
            head.clone()
        };
        want.push(format!("field|S::f{i}|{c}"));
    }
    src.push_str("}\n");
    let out = scan_fixture("o6", &[(LIB, &src)], &[]);
    let want: Vec<&str> = want.iter().map(String::as_str).collect();
    expect_sites(&out, LIB, &want);
}

#[test]
fn the_baseline_scanned_crates_are_workspace_members() {
    // Critique W4 (c): the scanned modes anchored in gate code must name real crates, or the
    // anchor is silently vacuous for the one it misspells.
    let manifest = fs::read_to_string(repo_root().join("Cargo.toml"))
        .expect("invariant: the root manifest exists");
    let members = scan::workspace_members(&manifest);
    for (path, _) in scan::BASELINE_SCANNED {
        assert!(
            *path == "." || members.iter().any(|m| m == path),
            "baseline crate {path} is not a workspace member"
        );
    }
    assert_eq!(
        scan::BASELINE_SCANNED.len(),
        23,
        "22 full crates and boyko_macros (emitted)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Controls: each a baseline-green / perturbed-red pair, on a fixture tree run through the whole
// gate. The baseline half proves the comparator can say green.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One fixture ledger row.
#[derive(Clone)]
struct RowSpec {
    owner: String,
    class: &'static str,
    form: String,
    destination: String,
    group: &'static str,
    shape: String,
    site: String,
}

/// Two `Vec` fields per group, so removing one row leaves every floor non-zero.
fn base_rows() -> Vec<RowSpec> {
    let mut v = Vec::new();
    for (g, group) in scan::GROUPS.iter().enumerate() {
        for k in 0..2 {
            let i = g * 2 + k;
            v.push(RowSpec {
                owner: format!("S::f{i}"),
                class: "B",
                form: "resource-column".to_owned(),
                destination: "fixture".to_owned(),
                group,
                shape: "field".to_owned(),
                site: format!("field|S::f{i}|Vec"),
            });
        }
    }
    v
}

fn base_lib() -> String {
    let mut s = String::from("pub struct S {\n");
    for i in 0..scan::GROUPS.len() * 2 {
        s.push_str(&format!("    pub f{i}: Vec<u8>,\n"));
    }
    s.push_str("}\n");
    // The own heads must name a type defined in a scanned crate; unit structs are not sites.
    for (head, origin) in scan::BASELINE_HEADS {
        if *origin == scan::Origin::Own {
            s.push_str(&format!("pub struct {head};\n"));
        }
    }
    s
}

fn ledger_text(rows: &[RowSpec]) -> String {
    let mut s = scan::TSV_HEADER.join("\t");
    s.push('\n');
    for r in rows {
        s.push_str(&format!(
            "c\t{LIB}\t{}\tfield\tVec\tu8\t{}\t{}\tnone\tonce\tno\t{}\t{}\t{}\t{}\n",
            r.owner, r.class, r.form, r.destination, r.group, r.shape, r.site
        ));
    }
    s
}

/// A pins line whose counts equal `rows`.
fn pins_line(rung: &str, kind: &str, rows: &[RowSpec], prev: &str, reason: &str) -> String {
    let oos = rows
        .iter()
        .filter(|r| r.form.starts_with("out-of-scope:"))
        .count();
    let semantic = rows
        .iter()
        .filter(|r| r.shape.starts_with("semantic:"))
        .count();
    let floors: Vec<String> = scan::GROUPS
        .iter()
        .map(|g| {
            let n = rows
                .iter()
                .filter(|r| r.group == *g && !r.shape.starts_with("semantic:"))
                .count();
            format!("{g}={n}")
        })
        .collect();
    format!(
        "{rung}\t{kind}\t{}\t{}\t{semantic}\t{oos}\t0\t0\t{}\t{prev}\t{reason}",
        rows.len() - oos,
        rows.len(),
        floors.join(";")
    )
}

struct Control {
    root: PathBuf,
}

impl Control {
    /// A consistent, green fixture tree.
    fn new(name: &str) -> Control {
        let root = tmp(name);
        write(
            &root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/c\"]\n",
        );
        write(&root, "crates/c/Cargo.toml", "[package]\nname = \"c\"\n");
        write(&root, LIB, &base_lib());
        write(
            &root,
            scan::CRATES_TSV,
            "kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\n",
        );
        write(&root, scan::VOCAB_TSV, "head\torigin\treason\n");
        let c = Control { root };
        c.ledger(&base_rows());
        c
    }

    /// Writes `rows` and a single pins line equal to them.
    fn ledger(&self, rows: &[RowSpec]) {
        write(&self.root, scan::LEDGER_TSV, &ledger_text(rows));
        let pins = format!(
            "{}\n{}\n",
            scan::PINS_HEADER.join("\t"),
            pins_line("F0", "pin", rows, "-", "fixture")
        );
        write(&self.root, scan::PINS_TSV, &pins);
    }

    fn run(&self) -> scan::GateReport {
        self.run_with(None, &self.root, &[])
    }

    fn run_with(
        &self,
        genesis: Option<&str>,
        scan_root: &Path,
        baseline: &[(&str, scan::Mode)],
    ) -> scan::GateReport {
        scan::run_gate(&scan::GateConfig {
            data_root: &self.root,
            scan_root,
            genesis,
            baseline,
            line_of: &no_lines,
        })
    }
}

fn assert_green(rep: &scan::GateReport) {
    assert!(
        rep.failures.is_empty(),
        "baseline half must be GREEN:\n{}",
        rep.render()
    );
    assert!(
        rep.compared > 0 && rep.sites > 0,
        "baseline half compared nothing"
    );
}

/// Prints a perturbed half's failures (captured unless `--nocapture`; the red-first receipts).
fn show_red(label: &str, rep: &scan::GateReport) {
    println!("--- {label}: perturbed half ---");
    for f in &rep.failures {
        println!("{f}");
    }
    println!("RESULT: RED ({} failures)", rep.failures.len());
}

fn assert_red_only(rep: &scan::GateReport, tag: &str, needle: &str) {
    show_red(&format!("expect one {tag} naming {needle}"), rep);
    let hits = rep.with_tag(tag);
    assert!(
        hits.len() == 1 && hits[0].contains(needle) && rep.failures.len() == 1,
        "expected exactly one `{tag}` failure naming `{needle}`, got:\n{}",
        rep.render()
    );
}

#[test]
fn control_extra_site_is_one_extra() {
    let c = Control::new("c_extra");
    assert_green(&c.run());
    let mut rows = base_rows();
    rows.remove(0);
    c.ledger(&rows);
    assert_red_only(&c.run(), "EXTRA", "S::f0");
}

#[test]
fn control_missing_site_is_one_missing() {
    let c = Control::new("c_missing");
    assert_green(&c.run());
    let mut rows = base_rows();
    let mut ghost = rows[0].clone();
    ghost.owner = "S::ghost".to_owned();
    ghost.site = "field|S::ghost|Vec".to_owned();
    rows.push(ghost);
    c.ledger(&rows);
    let rep = c.run();
    show_red("control_missing_site_is_one_missing", &rep);
    // A missing row is also a floor deficit of its group: the floor pins the comparison set.
    let missing = rep.with_tag("MISSING");
    let floors = rep.with_tag("FLOOR");
    assert!(
        missing.len() == 1
            && missing[0].contains("S::ghost")
            && floors.len() == 1
            && floors[0].contains(scan::GROUPS[0])
            && rep.failures.len() == 2,
        "expected one MISSING naming S::ghost and its group's FLOOR, got:
{}",
        rep.render()
    );
}

#[test]
fn control_empty_scan_dir_is_red_on_the_floor() {
    let c = Control::new("c_empty");
    assert_green(&c.run());
    let empty = tmp("c_empty_root");
    let rep = c.run_with(None, &empty, &[]);
    show_red("control_empty_scan_dir_is_red_on_the_floor", &rep);
    assert_eq!(
        rep.with_tag("FLOOR").len(),
        scan::GROUPS.len(),
        "every floor must be RED:\n{}",
        rep.render()
    );
}

#[test]
fn control_unparseable_file_is_red() {
    let c = Control::new("c_parse");
    assert_green(&c.run());
    write(&c.root, LIB, &format!("{}pub fn broken( {{\n", base_lib()));
    let rep = c.run();
    show_red("control_unparseable_file_is_red", &rep);
    assert!(!rep.with_tag("PARSE").is_empty(), "{}", rep.render());
}

#[test]
fn control_undeclared_member_is_red() {
    let c = Control::new("c_member");
    assert_green(&c.run());
    write(
        &c.root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/c\", \"crates/d\"]\n",
    );
    write(&c.root, "crates/d/Cargo.toml", "[package]\nname = \"d\"\n");
    assert_red_only(&c.run(), "MEMBER", "crates/d");
}

#[test]
fn control_pending_crate_that_is_a_member_is_red() {
    // Critique W5.
    let c = Control::new("c_pending");
    write(
        &c.root,
        scan::CRATES_TSV,
        "kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\ncrate\tcrates/d\tpending\tfixture\n",
    );
    assert_green(&c.run());
    write(
        &c.root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/c\", \"crates/d\"]\n",
    );
    write(&c.root, "crates/d/Cargo.toml", "[package]\nname = \"d\"\n");
    assert_red_only(&c.run(), "MEMBER", "pending");
}

#[test]
fn control_unscanned_runtime_dependency_is_red() {
    let c = Control::new("c_dep");
    write(
        &c.root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/c\", \"crates/u\"]\n",
    );
    write(&c.root, "crates/u/Cargo.toml", "[package]\nname = \"u\"\n");
    write(
        &c.root,
        scan::CRATES_TSV,
        "kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\ncrate\tcrates/u\tunscanned\tfixture\n",
    );
    assert_green(&c.run());
    write(
        &c.root,
        "crates/c/Cargo.toml",
        "[package]\nname = \"c\"\n\n[dependencies]\nu = { path = \"../u\" }\n",
    );
    assert_red_only(&c.run(), "DEP", "crates/u");
}

#[test]
fn control_undeclared_third_party_dependency_is_red() {
    // Critique O4.
    let c = Control::new("c_third");
    write(
        &c.root,
        "crates/c/Cargo.toml",
        "[package]\nname = \"c\"\n\n[dependencies]\nsmallvec = \"1\"\n",
    );
    assert_red_only(&c.run(), "THIRDPARTY", "smallvec");
    write(
        &c.root,
        scan::CRATES_TSV,
        "kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\nthird-party\tcrates/c\tsmallvec\tfixture: classified\n",
    );
    assert_green(&c.run());
}

#[test]
fn control_vocabulary_is_closed() {
    let c = Control::new("c_vocab");
    assert_green(&c.run());
    let mut rows = base_rows();
    rows[0].class = "U";
    c.ledger(&rows);
    assert_red_only(&c.run(), "VOCAB", "class `U`");
    let mut rows = base_rows();
    rows[0].form = "bogus".to_owned();
    c.ledger(&rows);
    assert_red_only(&c.run(), "VOCAB", "bogus");
}

#[test]
fn control_ruled_needs_a_keeping_ruling() {
    // Critique W4 (b): `out-of-scope:ruled` is tied to the closed list of rulings that keep a
    // site by design.
    let c = Control::new("c_ruled");
    let mut rows = base_rows();
    rows[0].form = "out-of-scope:ruled".to_owned();
    rows[0].destination = "U-5: kept by the ruling".to_owned();
    c.ledger(&rows);
    assert_green(&c.run());
    rows[0].destination = "U-7: not a keeping ruling".to_owned();
    c.ledger(&rows);
    assert_red_only(&c.run(), "VOCAB", "U-7");
}

#[test]
fn control_pin_raise_needs_merge_raise() {
    let c = Control::new("c_pin");
    let rows = base_rows();
    let mut fewer = rows.clone();
    fewer.pop();
    let first = pins_line("F0", "pin", &fewer, "-", "fixture");
    let raise = |kind: &str| {
        let second = pins_line(
            "F1",
            kind,
            &rows,
            &scan::fnv64(&first),
            "fixture: a merge added a row",
        );
        format!("{}\n{first}\n{second}\n", scan::PINS_HEADER.join("\t"))
    };
    write(&c.root, scan::PINS_TSV, &raise("merge-raise"));
    assert_green(&c.run());
    write(&c.root, scan::PINS_TSV, &raise("pin"));
    let rep = c.run();
    show_red("control_pin_raise_needs_merge_raise", &rep);
    assert!(
        !rep.with_tag("PIN").is_empty()
            && rep
                .failures
                .iter()
                .all(|f| f.starts_with("PIN") && f.contains("rises")),
        "a raise on a `pin` line must be RED:\n{}",
        rep.render()
    );
}

#[test]
fn control_pin_history_is_anchored() {
    // Critique W4 (a): the first line is anchored in gate code; later lines are hash-chained.
    let c = Control::new("c_anchor");
    let rows = base_rows();
    let first = pins_line("F0", "pin", &rows, "-", "fixture");
    assert_green(&c.run_with(Some(&first), &c.root, &[]));
    let rewritten = pins_line("F0", "pin", &rows, "-", "fixture, rewritten in place");
    write(
        &c.root,
        scan::PINS_TSV,
        &format!("{}\n{rewritten}\n", scan::PINS_HEADER.join("\t")),
    );
    assert_red_only(&c.run_with(Some(&first), &c.root, &[]), "PIN", "genesis");
    let second = pins_line("F1", "pin", &rows, "0000000000000000", "fixture");
    write(
        &c.root,
        scan::PINS_TSV,
        &format!("{}\n{first}\n{second}\n", scan::PINS_HEADER.join("\t")),
    );
    assert_red_only(&c.run_with(Some(&first), &c.root, &[]), "PIN", "hashes to");
}

#[test]
fn control_include_is_declared() {
    let c = Control::new("c_include");
    assert_green(&c.run());
    write(
        &c.root,
        LIB,
        &format!("{}include!(\"gen.rs\");\n", base_lib()),
    );
    assert_red_only(&c.run(), "INCLUDE", "gen.rs");
}

#[test]
fn control_orphan_file_is_red() {
    // Critique W3: a file under a scanned src/ that the walk does not reach.
    let c = Control::new("c_orphan");
    assert_green(&c.run());
    write(
        &c.root,
        "crates/c/src/orphan.rs",
        "pub struct O { v: Vec<u8> }\n",
    );
    assert_red_only(&c.run(), "ORPHAN", "orphan.rs");
    write(
        &c.root,
        scan::CRATES_TSV,
        "kind\tpath\tvalue\treason\ncrate\tcrates/c\tfull\tfixture\nunreached\tcrates/c/src/orphan.rs\t-\tfixture: declared\n",
    );
    assert_green(&c.run());
}

#[test]
fn control_baseline_mode_cannot_be_demoted() {
    // Critique W4 (c): the scanned modes of the baseline crates live in gate code.
    let c = Control::new("c_baseline");
    let baseline = [("crates/c", scan::Mode::Full)];
    assert_green(&c.run_with(None, &c.root, &baseline));
    write(
        &c.root,
        scan::CRATES_TSV,
        "kind\tpath\tvalue\treason\ncrate\tcrates/c\tunscanned\tfixture\n",
    );
    let rep = c.run_with(None, &c.root, &baseline);
    show_red("control_baseline_mode_cannot_be_demoted", &rep);
    assert!(
        rep.with_tag("MEMBER")
            .iter()
            .any(|f| f.contains("must stay")),
        "demoting a baseline crate must be RED:\n{}",
        rep.render()
    );
}
