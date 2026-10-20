//! Render wave PX1a test 2 (`plan/H4-PX1a-hdr-lit.md` Tests 2, rev 3 verify I2; `px1a/cut.md` C1, PC-PX1a-23;
//! orchestrator ruling 67 W5): **no `lit` producer tonemaps, and no `lit` store clamps.**
//!
//! After PX1a every `lit` producer stores pre-exposed scene-linear radiance and ONE pass,
//! `post_final`, applies the tonemap and the OETF. This census reads every shader source under
//! `crates/boyko_rhi_vulkan/shaders/` (comments stripped; the eDSL-owned `// === GENERATED … ===`
//! spans skipped) and holds the tree to three rules:
//!
//! 1. **The tail lives in `post_final` alone.** No shader but the generated `post_final.comp.hlsl`
//!    names a tail token ([`TAIL_TOKENS`]: `tonemap_select`, the hand curves, `OETF_GAMMA_EXP`) —
//!    a call, a definition or the OETF exponent. A file is named with every line that does.
//! 2. **No store clamps to display range.** At every `lit` store site — each `gLit[…] = <expr>;`
//!    in a compute shader, and each `return <expr>;` of `main` in a lit-attachment fragment shader
//!    ([`LIT_ATTACHMENT_FS`]) — the stored expression holds no `clamp(`, no `saturate(` and no
//!    `min(…, 1)`. Parsed per store site, not per file: a failure names the file and the line.
//! 3. **Every store saturates through the ONE helper (ruling 67 W5).** A narrowing store may round
//!    an out-of-range finite value to +Inf (Vulkan, "Floating-Point Format Conversions"), and every
//!    HDR reader reads a non-finite texel as a fault and zeroes the pixel — so an overflowing
//!    highlight would turn black. Each store expression therefore passes through
//!    [`STORE_HELPER`], which bounds a channel at `HDR_MAX` and keeps a NaN a NaN; it is defined
//!    exactly once, in `lit_format.hlsli`, inside an eDSL-generated span. **The stated exemption:**
//!    [`STORE_HELPER`] is the one bounded call rule 2 allows in a stored expression (it bounds at
//!    `HDR_MAX`, not at display white).
//!
//! The diagnostic magenta store `vb_shade.comp.hlsl` writes for a pixel no material covers is a
//! constant, carries no tail and no clamp, and is allowed by name ([`DIAGNOSTIC_STORES`],
//! PC-PX1a-23). The set of store sites is pinned per file ([`EXPECTED_STORES`]) so a new producer
//! is classified here before it ships, and every rule is also run against fixtures it must reject.

use std::fs;
use std::path::PathBuf;

/// Tokens that are the display-referred tail: the tonemapper dispatch, the hand curves and the
/// OETF exponent. Matched as whole identifiers.
const TAIL_TOKENS: &[&str] =
    &["tonemap_select", "aces_fitted", "khronos_pbr_neutral", "reinhard_jodie", "OETF_GAMMA_EXP"];

/// The one shader allowed to carry the tail: `post_final`, generated whole from
/// `boyko_shaderdsl::post` (`tests/post_edsl_sync.rs` byte-gates it).
const TAIL_HOME: &str = "post_final.comp.hlsl";

/// The fragment shaders whose colour output is a `lit` attachment (Forward's opaque draw, and the
/// sky shared by Forward and VB's `vb_sky`): their `main`'s returns are store sites.
const LIT_ATTACHMENT_FS: &[&str] = &["forward_opaque.fs.hlsl", "forward_sky.fs.hlsl"];

/// The producers' store helper (ruling 67 W5) — the stated exemption of rule 2.
const STORE_HELPER: &str = "lit_store_saturate";

/// The file that defines [`STORE_HELPER`], inside a generated span.
const STORE_HELPER_HOME: &str = "lit_format.hlsli";

/// Store sites allowed to bypass [`STORE_HELPER`]: `(file, stored expression with whitespace
/// removed, reason)`.
const DIAGNOSTIC_STORES: &[(&str, &str, &str)] = &[(
    "vb_shade.comp.hlsl",
    "float4(1.0,0.0,1.0,1.0)",
    "the diagnostic magenta for a pixel no material covers: a finite constant, no tail, no clamp",
)];

/// The `lit` store sites per file — compute `gLit[…] =` stores and lit-attachment FS returns.
const EXPECTED_STORES: &[(&str, usize)] = &[
    ("deferred_pbr.hlsl", 1),
    ("forward_opaque.fs.hlsl", 1),
    ("forward_sky.fs.hlsl", 1),
    ("sdf_forward_march.comp.hlsl", 1),
    ("vb_resolve.comp.hlsl", 1),
    ("vb_shade.comp.hlsl", 2),
    ("vb_shade_split.comp.hlsl", 1),
];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Every `.hlsl` / `.hlsli` file name under `shaders/`, sorted, with its text.
fn shader_sources() -> Vec<(String, String)> {
    let dir = shaders_dir();
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .filter(|p| p.extension().is_some_and(|x| x == "hlsl" || x == "hlsli"))
        .map(|p| {
            let name = p.file_name().expect("invariant: a file has a name").to_string_lossy().into_owned();
            let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("invariant: {} is UTF-8: {e}", p.display()));
            (name, text)
        })
        .collect();
    out.sort();
    out
}

/// `true` for a `// === GENERATED <name> BEGIN …` line (`END` for the closing sentinel).
fn is_sentinel(line: &str, word: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix("// === GENERATED ") else { return false };
    let mut parts = rest.split_whitespace();
    parts.next().is_some() && parts.next() == Some(word)
}

/// `text` with every eDSL-generated span (sentinels included) blanked line for line.
fn without_generated_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.lines() {
        if !inside && is_sentinel(line, "BEGIN") {
            inside = true;
        }
        if !inside {
            out.push_str(line);
        }
        if inside && is_sentinel(line, "END") {
            inside = false;
        }
        out.push('\n');
    }
    out
}

/// `text` with `//` and `/* */` comments replaced by spaces (newlines kept, so byte offsets and
/// line numbers survive).
fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            out.extend_from_slice(b"  ");
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            if i < b.len() {
                out.extend_from_slice(b"  ");
                i += 2;
            }
        } else if b[i] == b'"' {
            out.push(b'"');
            i += 1;
            while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                out.push(b[i]);
                i += 1;
            }
            if i < b.len() && b[i] == b'"' {
                out.push(b'"');
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("invariant: only ASCII bytes were replaced")
}

/// `src` with every `#else` / `#elif` arm of a conditional blanked (directive lines too), so a
/// construct a shader spells once per variant (`forward_opaque`'s `#ifdef FROXEL` loop header)
/// brace-matches. Lengths and newlines are kept, so offsets into it are offsets into `src`.
fn first_arm_only(src: &str) -> String {
    // One entry per open conditional: `true` while the current arm is NOT the first.
    let mut stack: Vec<bool> = Vec::new();
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        let directive = t.strip_prefix('#').map(str::trim_start);
        let blank = match directive {
            Some(d) if d.starts_with("if") => {
                stack.push(false);
                true
            }
            Some(d) if d.starts_with("el") => {
                if let Some(top) = stack.last_mut() {
                    *top = true;
                }
                true
            }
            Some(d) if d.starts_with("endif") => {
                stack.pop();
                true
            }
            _ => stack.iter().any(|&in_alt| in_alt),
        };
        if blank {
            out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// The source a rule reads: comments stripped, generated spans skipped.
fn scanned(text: &str) -> String {
    strip_comments(&without_generated_spans(text))
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Byte offsets of `word` as a whole identifier in `text`.
fn ident_offsets(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = text[from..].find(word) {
        let at = from + k;
        let end = at + word.len();
        if (at == 0 || !is_ident(b[at - 1])) && (end >= b.len() || !is_ident(b[end])) {
            out.push(at);
        }
        from = end;
    }
    out
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// The offset of the bracket matching the opener at `open` (`(`, `[` or `{`).
fn matching(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let (o, c) = match b[open] {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0usize;
    for (k, &ch) in b.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// One `lit` store site: its line and the stored expression.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoreSite {
    line: usize,
    expr: String,
}

/// Every `gLit[…] = <expr>;` in `src` (already scanned).
fn compute_store_sites(src: &str) -> Vec<StoreSite> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    for at in ident_offsets(src, "gLit") {
        let mut k = at + "gLit".len();
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= b.len() || b[k] != b'[' {
            continue;
        }
        let Some(close) = matching(src, k) else { continue };
        let mut e = close + 1;
        while e < b.len() && b[e].is_ascii_whitespace() {
            e += 1;
        }
        if e + 1 >= b.len() || b[e] != b'=' || b[e + 1] == b'=' {
            continue;
        }
        let semi = src[e..].find(';').map(|s| e + s).unwrap_or(b.len());
        out.push(StoreSite { line: line_of(src, at), expr: src[e + 1..semi].trim().to_string() });
    }
    out
}

/// Every `return <expr>;` inside the body of the function named `main` in `src` (already scanned).
fn fs_main_return_sites(src: &str) -> Vec<StoreSite> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    for at in ident_offsets(src, "main") {
        let mut k = at + "main".len();
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k >= b.len() || b[k] != b'(' {
            continue;
        }
        let Some(params_end) = matching(src, k) else { continue };
        // A definition's parameter list is followed (past semantics such as `: SV_Target`) by `{`
        // before any `;`.
        let rest = &src[params_end + 1..];
        let (Some(brace), semi) = (rest.find('{'), rest.find(';')) else { continue };
        if semi.is_some_and(|s| s < brace) {
            continue;
        }
        let open = params_end + 1 + brace;
        // Braces are matched on the first arm of every conditional; returns are read from all arms.
        let Some(close) = matching(&first_arm_only(src), open) else { continue };
        let body = &src[open..close];
        for r in ident_offsets(body, "return") {
            let semi = body[r..].find(';').map(|s| r + s).unwrap_or(body.len());
            out.push(StoreSite { line: line_of(src, open + r), expr: body[r + "return".len()..semi].trim().to_string() });
        }
    }
    out
}

/// The store sites of one file: compute stores, plus `main`'s returns for a lit-attachment FS.
fn store_sites(file: &str, src: &str) -> Vec<StoreSite> {
    let mut sites = compute_store_sites(src);
    if LIT_ATTACHMENT_FS.contains(&file) {
        sites.extend(fs_main_return_sites(src));
    }
    sites
}

/// The display-range bound an expression applies, if any: `clamp(`, `saturate(`, or `min(` with
/// a literal-one argument. [`STORE_HELPER`] is the stated exemption and is masked first.
fn display_clamp_in(expr: &str) -> Option<&'static str> {
    let masked = expr.replace(STORE_HELPER, "__store_helper__");
    let called = |word: &str| {
        ident_offsets(&masked, word).into_iter().find(|&at| masked[at + word.len()..].trim_start().starts_with('('))
    };
    if called("clamp").is_some() {
        return Some("clamp(");
    }
    if called("saturate").is_some() {
        return Some("saturate(");
    }
    for at in ident_offsets(&masked, "min") {
        let rest = &masked[at + 3..];
        let Some(open_rel) = rest.find('(') else { continue };
        if !rest[..open_rel].trim().is_empty() {
            continue;
        }
        let open = at + 3 + open_rel;
        let Some(close) = matching(&masked, open) else { continue };
        if top_level_args(&masked[open + 1..close]).iter().any(|a| is_literal_one(a)) {
            return Some("min(…, 1)");
        }
    }
    None
}

/// The comma-separated arguments of a call's argument text, split at depth 0.
fn top_level_args(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (k, ch) in args.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[start..k].trim());
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(args[start..].trim());
    out
}

fn is_literal_one(arg: &str) -> bool {
    let a = arg.trim().trim_end_matches(['f', 'F', 'h', 'H']);
    a == "1" || (a.starts_with("1.") && a[2..].bytes().all(|c| c == b'0'))
}

fn compact(expr: &str) -> String {
    expr.chars().filter(|c| !c.is_whitespace()).collect()
}

fn is_diagnostic(file: &str, expr: &str) -> bool {
    let c = compact(expr);
    DIAGNOSTIC_STORES.iter().any(|&(f, e, _)| f == file && e == c)
}

/// Rule 1 over one file: the line of every tail token.
fn tail_lines(src: &str) -> Vec<usize> {
    let mut lines: Vec<usize> =
        TAIL_TOKENS.iter().flat_map(|t| ident_offsets(src, t)).map(|at| line_of(src, at)).collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

#[test]
fn no_shader_outside_post_final_carries_the_tail() {
    let sources = shader_sources();
    assert!(sources.len() >= 90, "the census found only {} shader sources — wrong directory?", sources.len());
    assert!(sources.iter().any(|(n, _)| n == TAIL_HOME), "{TAIL_HOME} is missing: the tail has no home");
    let offenders: Vec<String> = sources
        .iter()
        .filter(|(name, _)| name != TAIL_HOME)
        .filter_map(|(name, text)| {
            let lines = tail_lines(&scanned(text));
            (!lines.is_empty()).then(|| format!("{name} (lines {lines:?})"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{} shader(s) outside {TAIL_HOME} carry the display tail (tonemap / OETF): {offenders:#?}",
        offenders.len()
    );
}

#[test]
fn no_lit_store_clamps_to_display_range() {
    let mut offenders = Vec::new();
    for (name, text) in shader_sources() {
        for site in store_sites(&name, &scanned(&text)) {
            if let Some(kind) = display_clamp_in(&site.expr) {
                offenders.push(format!("{name}:{} stores `{}` ({kind})", site.line, site.expr));
            }
        }
    }
    assert!(offenders.is_empty(), "{} lit store site(s) clamp to display range: {offenders:#?}", offenders.len());
}

#[test]
fn the_lit_store_sites_are_exactly_the_pinned_producers() {
    let mut found: Vec<(String, usize)> = shader_sources()
        .into_iter()
        .map(|(name, text)| {
            let n = store_sites(&name, &scanned(&text)).len();
            (name, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    found.sort();
    let expected: Vec<(String, usize)> = EXPECTED_STORES.iter().map(|&(f, n)| (f.to_string(), n)).collect();
    assert_eq!(found, expected, "the lit store sites moved: classify the new producer in this census");
}

#[test]
fn every_lit_store_saturates_through_the_one_helper() {
    let mut offenders = Vec::new();
    for (name, text) in shader_sources() {
        for site in store_sites(&name, &scanned(&text)) {
            if is_diagnostic(&name, &site.expr) {
                continue;
            }
            if ident_offsets(&site.expr, STORE_HELPER).is_empty() {
                offenders.push(format!("{name}:{} stores `{}` without {STORE_HELPER}", site.line, site.expr));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "{} lit store site(s) do not bound an overflowing highlight at HDR_MAX: {offenders:#?}",
        offenders.len()
    );
}

#[test]
fn the_store_helper_is_defined_once_inside_a_generated_span() {
    let sources = shader_sources();
    let home = sources
        .iter()
        .find(|(n, _)| n == STORE_HELPER_HOME)
        .unwrap_or_else(|| panic!("no {STORE_HELPER_HOME}: the store helper has no home"));
    let defined_in = |text: &str| {
        let src = strip_comments(text);
        ident_offsets(&src, STORE_HELPER).into_iter().any(|at| {
            let before = src[..at].trim_end();
            before.ends_with("float3") || before.ends_with("float4") || before.ends_with("float")
        })
    };
    // Outside the generated span the home must not define it (a hand copy beside the generated one).
    assert!(defined_in(&home.1), "{STORE_HELPER_HOME} does not define {STORE_HELPER}");
    assert!(!defined_in(&without_generated_spans(&home.1)), "{STORE_HELPER} is defined outside a generated span");
    let others: Vec<&str> =
        sources.iter().filter(|(n, t)| n != STORE_HELPER_HOME && defined_in(t)).map(|(n, _)| n.as_str()).collect();
    assert!(others.is_empty(), "{STORE_HELPER} is defined again in {others:?}");
}

/// Every rule, run against fixtures it must reject and accept — so no rule is vacuous.
#[test]
fn the_rules_reject_their_fixtures() {
    let tail = "float3 f(float3 c) {\n    return pow(tonemap_select(c, 0u), OETF_GAMMA_EXP);\n}\n";
    assert_eq!(tail_lines(&scanned(tail)), [2]);
    assert!(tail_lines(&scanned("// tonemap_select( in a comment\nfloat x = my_tonemap_select2;\n")).is_empty());
    let generated = "// === GENERATED post_x BEGIN ===\nfloat3 t = tonemap_select(c, 0u);\n// === GENERATED post_x END ===\n";
    assert!(tail_lines(&scanned(generated)).is_empty(), "a generated span is the eDSL's");

    let stores = "void main() {\n  gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);\n  gLit[p] = float4(saturate(lit), 1.0);\n  gLit[p] = float4(min(lit, 1.0), 1.0);\n  gLit[p] = float4(min(lit, HDR_MAX), 1.0);\n  if (gLit[p].x == 0.0) {}\n}\n";
    let sites = compute_store_sites(&scanned(stores));
    assert_eq!(sites.len(), 4, "{sites:?}");
    let kinds: Vec<Option<&str>> = sites.iter().map(|s| display_clamp_in(&s.expr)).collect();
    assert_eq!(kinds, [Some("clamp("), Some("saturate("), Some("min(…, 1)"), None]);
    assert_eq!(display_clamp_in("float4(lit_store_saturate(lit), 1.0)"), None, "the stated exemption");
    assert_eq!(display_clamp_in("float4(my_clamp(lit), 1.0)"), None);

    let fs = "float4 helper() { return float4(1, 1, 1, 1); }\nfloat4 main(float4 p : SV_Position) : SV_Target0 {\n  if (a) { return float4(clamp(x, 0.0, 1.0), 1.0); }\n#ifdef V\n  for (;;) {\n#else\n  for (;;) {\n#endif\n  }\n  return float4(lit_store_saturate(x), 1.0);\n}\n";
    let rets = fs_main_return_sites(&scanned(fs));
    assert_eq!(rets.len(), 2, "{rets:?}");
    assert_eq!(rets[0].line, 3);
    assert_eq!(display_clamp_in(&rets[0].expr), Some("clamp("));
    assert!(!ident_offsets(&rets[1].expr, STORE_HELPER).is_empty());

    assert!(is_diagnostic("vb_shade.comp.hlsl", "float4(1.0, 0.0, 1.0, 1.0)"));
    assert!(!is_diagnostic("vb_resolve.comp.hlsl", "float4(1.0, 0.0, 1.0, 1.0)"));
}
