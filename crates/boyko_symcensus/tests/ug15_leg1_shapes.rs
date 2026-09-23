//! The leg-(1) instrument's own fixture table (cut §4.1 anti-vacuity): every counted form must
//! be found by the `syn` visitor AND by M-a inside a `macro_rules!` body, and every non-counted
//! form must be found by neither. A visitor that finds nothing passes every census; this table is
//! what makes that impossible.

use proc_macro2::TokenStream;
use std::str::FromStr;

use boyko_symcensus::source::{self, Counted, Via};

/// `(fixture source, the one counted form it holds)`.
const COUNTED: &[(&str, &str)] = &[
    ("#[no_mangle] pub fn f() {}", "#[no_mangle]"),
    ("#[unsafe(no_mangle)] pub fn f() {}", "#[no_mangle]"),
    ("#[export_name = \"x\"] pub fn f() {}", "#[export_name]"),
    ("#[unsafe(export_name = \"x\")] pub fn f() {}", "#[export_name]"),
    ("#[unsafe(link_section = \".x\")] static S: u8 = 0;", "#[link_section]"),
    ("#[used] static S: u8 = 0;", "#[used]"),
    ("#[used(linker)] static S: u8 = 0;", "#[used]"),
    ("#[linkage = \"weak\"] static S: u8 = 0;", "#[linkage]"),
    ("#[cfg_attr(windows, unsafe(no_mangle))] pub fn f() {}", "#[no_mangle]"),
    ("#[cfg_attr(windows, cfg_attr(unix, used))] static S: u8 = 0;", "#[used]"),
    ("#[cfg_attr(windows, inline, unsafe(link_section = \".x\"))] static S: u8 = 0;", "#[link_section]"),
    ("pub extern \"C\" fn f() {}", "extern \"C\" fn"),
    ("extern \"system\" fn f() {}", "extern \"system\" fn"),
    ("pub extern \"C-unwind\" fn f() {}", "extern \"C-unwind\" fn"),
    ("extern \"sysv64\" fn f() {}", "extern \"sysv64\" fn"),
    ("extern \"win64\" fn f() {}", "extern \"win64\" fn"),
    ("pub unsafe extern \"C\" fn f() {}", "extern \"C\" fn"),
    ("extern fn f() {}", "extern fn (bare)"),
    ("struct T; impl T { pub extern \"C\" fn f() {} }", "extern \"C\" fn"),
    ("trait Tr { extern \"C\" fn f() {} }", "extern \"C\" fn"),
    ("struct T; trait Tr { extern \"C\" fn f(); } impl Tr for T { extern \"C\" fn f() {} }", "extern \"C\" fn"),
    ("core::arch::global_asm!(\"nop\");", "global_asm!"),
    ("global_asm!(\"nop\");", "global_asm!"),
    ("#[naked] pub extern \"sysv64\" fn f() { naked_asm!(\"ret\") }", "#[naked]"),
    ("#[unsafe(naked)] pub fn f() { core::arch::naked_asm!(\"ret\") }", "#[naked]"),
    ("pub fn f() { core::arch::naked_asm!(\"ret\") }", "naked_asm!"),
    ("fn outer() { #[unsafe(no_mangle)] fn inner() {} }", "#[no_mangle]"),
    ("const _: () = { #[used] static S: u8 = 0; };", "#[used]"),
    ("mod m { mod n { #[unsafe(export_name = \"y\")] fn f() {} } }", "#[export_name]"),
];

/// Forms that must NOT be counted.
const NOT_COUNTED: &[&str] = &[
    "extern \"C\" { fn f(); }",
    "unsafe extern \"system\" { pub fn GetTickCount() -> u32; }",
    "type F = extern \"C\" fn();",
    "static P: Option<unsafe extern \"C\" fn(u32) -> u32> = None;",
    "extern \"Rust\" fn f() {}",
    "extern crate core;",
    "fn f() { unsafe { core::arch::asm!(\"nop\") } }",
    "#[doc = \"no_mangle, used, link_section\"] fn f() {}",
    "#[allow(unused)] fn f() {}",
    "#[inline(never)] #[cold] fn f() {}",
    "trait Tr { extern \"C\" fn f(); }",
];

fn counted_names(v: &[Counted]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

fn syn_walk(src: &str) -> Vec<String> {
    let parsed = syn::parse_file(src).unwrap_or_else(|e| panic!("fixture does not parse: {src}: {e}"));
    let (findings, stats) = source::count_file("fixture.rs", &parsed, Via::SynItem);
    assert!(stats.items > 0 || src.contains('!'), "the visitor visited no item of `{src}`");
    findings.iter().map(|f| f.kind.to_string()).collect()
}

fn in_macro_rules(src: &str) -> Vec<String> {
    let wrapped = format!("macro_rules! m {{ () => {{ {src} }}; }}");
    let parsed = syn::parse_file(&wrapped).unwrap_or_else(|e| panic!("wrapped fixture does not parse: {wrapped}: {e}"));
    let (findings, stats) = source::count_file("fixture.rs", &parsed, Via::SynItem);
    assert_eq!(stats.macro_rules, 1, "the walker did not see the macro_rules! body of `{wrapped}`");
    findings.iter().filter(|f| f.via == Via::MacroTokens).map(|f| f.kind.to_string()).collect()
}

#[test]
fn every_counted_form_is_found_by_the_visitor_and_by_m_a() {
    let mut rows = 0;
    for (src, want) in COUNTED {
        let via_syn = syn_walk(src);
        assert!(via_syn.iter().any(|k| k == want), "syn visitor missed {want} in `{src}` (found {via_syn:?})");
        let via_ma = in_macro_rules(src);
        assert!(via_ma.iter().any(|k| k == want), "M-a missed {want} inside a macro_rules! body: `{src}` (found {via_ma:?})");
        let direct = counted_names(&source::scan_tokens(TokenStream::from_str(src).expect("tokenizes")));
        assert!(direct.iter().any(|k| k == want), "scan_tokens missed {want} in `{src}` (found {direct:?})");
        rows += 1;
    }
    println!("leg (1) shapes: {rows} counted form(s), each found three ways");
    assert_eq!(rows, COUNTED.len());
}

#[test]
fn no_non_counted_form_is_found() {
    for src in NOT_COUNTED {
        let via_syn = syn_walk(src);
        assert!(via_syn.is_empty(), "syn visitor counted `{src}`: {via_syn:?}");
        let via_ma = in_macro_rules(src);
        assert!(via_ma.is_empty(), "M-a counted `{src}` inside a macro_rules! body: {via_ma:?}");
    }
    println!("leg (1) shapes: {} non-counted form(s), none found", NOT_COUNTED.len());
}

#[test]
fn quote_interpolation_is_not_matched_but_a_named_template_fn_is() {
    // `#[#attr]`: the attribute is computed, which is exactly what M-a cannot see (control vi).
    let interp = source::scan_tokens(TokenStream::from_str("#[#attr] static S: u8 = 0;").expect("tokenizes"));
    assert!(interp.is_empty(), "M-a matched a quote interpolation: {interp:?}");
    // `extern "C" fn #name()`: a DEFINITION whose name is interpolated is counted…
    let named = counted_names(&source::scan_tokens(TokenStream::from_str("pub extern \"C\" fn #name() {}").expect("tokenizes")));
    assert_eq!(named, ["extern \"C\" fn"]);
    // …and so is a `macro_rules!` metavariable name; a fn-pointer TYPE is not.
    let meta = counted_names(&source::scan_tokens(TokenStream::from_str("extern \"C\" fn $name() {}").expect("tokenizes")));
    assert_eq!(meta, ["extern \"C\" fn"]);
    let fnptr = source::scan_tokens(TokenStream::from_str("let f: extern \"C\" fn() = g;").expect("tokenizes"));
    assert!(fnptr.is_empty(), "M-a matched a fn-pointer type: {fnptr:?}");
}

#[test]
fn build_rs_strings_are_scanned_as_tokens_or_as_text() {
    let src = r##"
        fn main() {
            let a = "#[unsafe(no_mangle)] pub fn generated() {}";
            let b = format!("pub const X: u32 = {};", 1);
            let c = "`not tokens` #[used] static G: u8 = 0;";
            let d = "a sentence that uses the word used";
        }
    "##;
    let (findings, stats) = source::scan_build_rs("build.rs", src).unwrap_or_else(|r| panic!("{r}"));
    let kinds: Vec<String> = findings.iter().map(|f| f.kind.to_string()).collect();
    assert_eq!(stats.build_rs_strings, 4, "every string literal is read, including format! arguments");
    assert_eq!(kinds, ["#[no_mangle]", "#[used]"], "one tokenized hit and one text hit, and no hit on prose");
}

#[test]
fn allowlist_parser_refuses_what_it_does_not_understand() {
    assert!(source::parse_allowlist("# empty\n").unwrap_or_else(|r| panic!("{r}")).is_empty());
    let one = "[[allow]]\nfile = \"a.rs\"\nitem = \"f\"\nkind = \"#[used]\"\nreason = \"owner: x\"\n";
    assert_eq!(source::parse_allowlist(one).unwrap_or_else(|r| panic!("{r}")).len(), 1);
    assert!(source::parse_allowlist("file = \"a.rs\"\n").is_err(), "a key outside a table");
    assert!(source::parse_allowlist("[[allow]]\nfile = \"a.rs\"\n").is_err(), "a table without item/kind/reason");
    assert!(source::parse_allowlist("[[allow]]\nfile = a.rs\nitem = \"f\"\nkind = \"k\"\nreason = \"r\"\n").is_err(), "an unquoted value");
    assert!(source::parse_allowlist("[allow]\n").is_err(), "a single-bracket table");
}
