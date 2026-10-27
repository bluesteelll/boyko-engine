//! Render rung GB, gate **G1** (device-free): the pass names the GPU crash diagnostics label and
//! report are well formed, and every GB recording site has the shape the zero-cost claim needs.
//!
//! # What it reads, and why a source census
//!
//! The production declarators are `&mut self` `Renderer` methods that need a device; no
//! device-free harness drives them (`framegraph_gbuffer_equiv.rs` re-declares by hand), and two
//! pass names repeat within one frame by design (`ssao_atrous` and `shadow_atrous` are declared
//! once per à-trous level). So the per-frame parity — every declared pass enters exactly one scope
//! — is gate G2's, on a device, and this gate holds the SOURCE to what G2 and the report rely on:
//!
//! - **(a)** every `add_pass(` argument under `crates/boyko_rhi_vulkan/src` (test modules
//!   excluded) is a string literal, or an index into a `const` array of string literals, and
//!   every such name matches `^[a-z][a-z0-9_]{0,62}$` — a label is a ≤ 63-byte copy + NUL, and
//!   the report prints names unquoted;
//! - **(b)** each of the three barrier funnels (`record_graph_pass`, `record_forward_pass`,
//!   `record_vb_pass`) calls `crumb_enter_pass` exactly once, before its `record_pass`;
//! - **(c)** each of the three recorders (`gbuffer.rs`, `forward.rs`, `vb.rs`) calls
//!   `crumb_frame_close` exactly once, and it is the last call before `end_command_buffer`;
//! - **(d)** the host scopes exist: FXAA in `record_fxaa`, SMAA in `record_smaa`, the present
//!   blit in each recorder, each before its zone / body;
//! - **(e)** every call of a GB recording method anywhere in the crate's source is the
//!   single-branch form — the line before it is `if self.crumbs_armed() {` (or `this.`) — and
//!   every such method is `#[cold] #[inline(never)]`, and `crumbs_armed` is the
//!   `COMPILED_IN && crumb_bits != 0` byte test;
//! - **(f)** it prints what it counted, and a census that matched nothing is RED, not green.
//!
//! The scanner's own red is shown by the self-test at the bottom: the three malformed `add_pass`
//! fixtures and a funnel with its `crumb_enter_pass` removed are each refused.

use std::fs;
use std::path::{Path, PathBuf};

/// The GB recording methods: each must be `#[cold] #[inline(never)]` and each call must sit inside
/// `if self.crumbs_armed() {`.
const SITE_FNS: &[&str] = &[
    "crumb_begin_frame",
    "crumb_enter_pass",
    "crumb_enter_host",
    "crumb_frame_close",
    "crumb_verify_slot",
];

/// The three barrier funnels, in `present/graph_bridge.rs`.
const FUNNELS: &[&str] = &["record_graph_pass", "record_forward_pass", "record_vb_pass"];

/// The three recorders, with the function that records the frame.
const RECORDERS: &[(&str, &str)] = &[
    ("present/passes/gbuffer.rs", "record_gbuffer"),
    ("present/passes/forward.rs", "record_forward"),
    ("present/passes/vb.rs", "record_vb"),
];

/// The literal `add_pass` sites outside test modules at the rung that added this census (82 in
/// `graph_bridge.rs` + 1 in `frame_driver.rs`; `graph_bridge.rs`'s test module holds 7 more);
/// fewer means the scanner went blind.
const MIN_LITERAL_PASS_NAMES: usize = 83;

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The file's lines with every `#[cfg(test)]` item blanked (a test module records no frame);
/// line numbers are preserved.
fn non_test_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth: Option<i32> = None;
    let mut pending = false;
    for line in text.lines() {
        if let Some(d) = depth.as_mut() {
            *d += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if *d <= 0 {
                depth = None;
            }
            out.push(String::new());
            continue;
        }
        let t = line.trim();
        if t == "#[cfg(test)]" {
            pending = true;
            out.push(String::new());
            continue;
        }
        if pending && line.contains('{') {
            pending = false;
            let d = line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if d > 0 {
                depth = Some(d);
            }
            out.push(String::new());
            continue;
        }
        if pending && (t.ends_with(';') || t.starts_with("mod ")) {
            pending = false;
            out.push(String::new());
            continue;
        }
        out.push(line.to_string());
    }
    out
}

/// The code part of a line (a `//` comment cut off; good enough for this tree, whose `add_pass`
/// and GB call lines carry no `//` inside a string).
fn code(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn is_pass_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// The string literals of `const NAME: [&str; …] = [ … ];` in `text`, or `None` if no such const.
fn const_str_array(text: &str, name: &str) -> Option<Vec<String>> {
    let head = format!("const {name}: [&str;");
    let at = text.find(&head)?;
    let eq = at + text[at..].find('=')?;
    let open = eq + text[eq..].find('[')? + 1;
    let close = open + text[open..].find("];")?;
    let body = &text[open..close];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(q) = rest.find('"') {
        let tail = &rest[q + 1..];
        let end = tail.find('"')?;
        out.push(tail[..end].to_string());
        rest = &tail[end + 1..];
    }
    Some(out)
}

/// Check (a) over one file's non-test lines. Returns `(literal names, problems)`.
fn check_pass_names(file: &str, lines: &[String], full_text: &str) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut problems = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let c = code(line);
        let mut from = 0;
        while let Some(rel) = c[from..].find("add_pass(") {
            let at = from + rel + "add_pass(".len();
            from = at;
            // The definition `pub fn add_pass(&mut self, name: …)` is not a call.
            if c[..at].trim_end_matches("add_pass(").trim_end().ends_with("fn") {
                continue;
            }
            let Some(end) = c[at..].find(')') else {
                problems.push(format!("{file}:{}: add_pass( with its argument on another line", i + 1));
                continue;
            };
            let arg = c[at..at + end].trim();
            if let Some(lit) = arg.strip_prefix('"').and_then(|a| a.strip_suffix('"')) {
                if is_pass_name(lit) {
                    names.push(lit.to_string());
                } else {
                    problems.push(format!("{file}:{}: pass name {arg} does not match ^[a-z][a-z0-9_]{{0,62}}$", i + 1));
                }
            } else if let Some((array, _)) = arg.split_once('[') {
                match const_str_array(full_text, array.trim()) {
                    Some(items) if !items.is_empty() => {
                        for it in items {
                            if !is_pass_name(&it) {
                                problems.push(format!("{file}:{}: {array}[..] holds \"{it}\", not a pass name", i + 1));
                            }
                        }
                    }
                    _ => problems.push(format!(
                        "{file}:{}: add_pass({arg}) indexes `{array}`, which is not a const array of string literals in this file",
                        i + 1
                    )),
                }
            } else {
                problems.push(format!(
                    "{file}:{}: add_pass({arg}) — a pass name must be a literal or a const array of literals",
                    i + 1
                ));
            }
        }
    }
    (names, problems)
}

/// The body of `fn <name>(` in `lines`: from the signature to the closing brace at the
/// signature's depth. `None` if the fn is absent.
fn fn_body(lines: &[String], name: &str) -> Option<(usize, Vec<String>)> {
    let sig = format!("fn {name}(");
    let start = lines.iter().position(|l| code(l).contains(&sig))?;
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut body = Vec::new();
    for l in &lines[start..] {
        let c = code(l);
        depth += c.matches('{').count() as i32 - c.matches('}').count() as i32;
        if c.contains('{') {
            seen_open = true;
        }
        body.push(l.clone());
        if seen_open && depth <= 0 {
            break;
        }
    }
    Some((start, body))
}

/// The index of the first line in `body` containing `needle` in code, and the count.
fn find_all(body: &[String], needle: &str) -> Vec<usize> {
    body.iter().enumerate().filter(|(_, l)| code(l).contains(needle)).map(|(i, _)| i).collect()
}

/// Check (b) over `graph_bridge.rs`'s lines.
fn check_funnels(lines: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    for funnel in FUNNELS {
        let Some((_, body)) = fn_body(lines, funnel) else {
            problems.push(format!("{funnel}: not found in present/graph_bridge.rs"));
            continue;
        };
        let enters = find_all(&body, "crumb_enter_pass(");
        let records = find_all(&body, ".record_pass(");
        if enters.len() != 1 {
            problems.push(format!("{funnel}: {} crumb_enter_pass, want 1", enters.len()));
            continue;
        }
        match records.first() {
            Some(&r) if enters[0] < r => {}
            _ => problems.push(format!("{funnel}: crumb_enter_pass is not before its record_pass")),
        }
    }
    problems
}

/// Check (c) and the present-blit half of (d) over one recorder.
fn check_recorder(file: &str, fn_name: &str, lines: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let Some((_, body)) = fn_body(lines, fn_name) else {
        return vec![format!("{file}: fn {fn_name} not found")];
    };
    let closes = find_all(&body, "crumb_frame_close(");
    let ends = find_all(&body, "end_command_buffer)(");
    if closes.len() != 1 {
        problems.push(format!("{file}: {} crumb_frame_close, want 1", closes.len()));
    } else if let Some(&end) = ends.last() {
        // Between the close and the end: only the guard's `}`, blank lines and comments.
        let between = &body[closes[0] + 1..end];
        let stray = between.iter().any(|l| {
            let c = code(l).trim();
            !(c.is_empty() || c == "}" || c.starts_with("let raw = unsafe"))
        });
        if closes[0] > end || stray {
            problems.push(format!("{file}: crumb_frame_close is not the last call before end_command_buffer"));
        }
    } else {
        problems.push(format!("{file}: no end_command_buffer in {fn_name}"));
    }
    let blits = find_all(&body, "HostScope::PresentBlit");
    let zone = find_all(&body, "let present_zone =");
    match (blits.as_slice(), zone.first()) {
        ([b], Some(&z)) if *b < z => {}
        _ => problems.push(format!("{file}: the PresentBlit host scope must appear once, before the present zone")),
    }
    problems
}

/// Check (d) for the two AA passes: the host scope is the first statement of the record fn.
fn check_aa(file: &str, fn_name: &str, scope: &str, lines: &[String]) -> Vec<String> {
    let Some((_, body)) = fn_body(lines, fn_name) else {
        return vec![format!("{file}: fn {fn_name} not found")];
    };
    let hits = find_all(&body, &format!("HostScope::{scope}"));
    if hits.len() != 1 {
        return vec![format!("{file}: {} HostScope::{scope} scopes in {fn_name}, want 1", hits.len())];
    }
    let first_stmt = body.iter().position(|l| code(l).trim_start().starts_with("if self.crumbs_armed()"));
    let before_any_record = body[..hits[0]].iter().all(|l| !code(l).contains(")(cmd"));
    if first_stmt.is_none() || !before_any_record {
        return vec![format!("{file}: the {scope} host scope must open before {fn_name} records anything")];
    }
    Vec::new()
}

/// Check (e): every call of a site fn is guarded; returns `(sites, problems)`.
fn check_guards(file: &str, lines: &[String]) -> (usize, Vec<String>) {
    let mut sites = 0;
    let mut problems = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let c = code(line);
        for f in SITE_FNS {
            let call_self = format!("self.{f}(");
            let call_this = format!("this.{f}(");
            if !(c.contains(&call_self) || c.contains(&call_this)) {
                continue;
            }
            sites += 1;
            let prev = lines[..i].iter().rev().map(|l| code(l).trim()).find(|t| !t.is_empty()).unwrap_or("");
            if prev != "if self.crumbs_armed() {" && prev != "if this.crumbs_armed() {" {
                problems.push(format!("{file}:{}: {f} is not inside `if <self>.crumbs_armed() {{` (previous code line: `{prev}`)", i + 1));
            }
        }
    }
    (sites, problems)
}

/// Check (e)'s other half: each site fn is declared `#[cold] #[inline(never)]`, and the gate is
/// the byte test.
fn check_cold_bodies(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut problems = Vec::new();
    for f in SITE_FNS {
        let sig = format!("fn {f}(");
        let Some(at) = lines.iter().position(|l| l.contains(&sig)) else {
            problems.push(format!("present/gpu_crumbs.rs: fn {f} not found"));
            continue;
        };
        let attrs: Vec<&str> = lines[..at].iter().rev().take(3).map(|l| l.trim()).collect();
        if !(attrs.contains(&"#[cold]") && attrs.contains(&"#[inline(never)]")) {
            problems.push(format!("present/gpu_crumbs.rs: fn {f} is not #[cold] #[inline(never)]"));
        }
    }
    let gate = "GpuCrashDiagLevel::COMPILED_IN && self.crumb_bits != 0";
    match text.find("fn crumbs_armed(&self) -> bool {") {
        Some(at) if text[at..].lines().nth(1).is_some_and(|l| l.trim() == gate) => {}
        _ => problems.push(format!("present/gpu_crumbs.rs: crumbs_armed must be exactly `{gate}`")),
    }
    problems
}

fn read(rel: &str) -> String {
    let p = src_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", p.display()))
}

#[test]
fn gb_sites_and_pass_names_are_well_formed() {
    let root = src_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    let mut problems = Vec::new();
    let mut names = Vec::new();
    let mut guarded_sites = 0usize;
    for path in &files {
        let rel = path.strip_prefix(&root).expect("under src").to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("invariant: {rel} is readable: {e}"));
        let lines = non_test_lines(&text);
        let (n, p) = check_pass_names(&rel, &lines, &text);
        names.extend(n);
        problems.extend(p);
        let (s, p) = check_guards(&rel, &lines);
        guarded_sites += s;
        problems.extend(p);
    }

    let bridge = non_test_lines(&read("present/graph_bridge.rs"));
    problems.extend(check_funnels(&bridge));
    for (file, f) in RECORDERS {
        problems.extend(check_recorder(file, f, &non_test_lines(&read(file))));
    }
    problems.extend(check_aa("present/passes/fxaa.rs", "record_fxaa", "Fxaa", &non_test_lines(&read("present/passes/fxaa.rs"))));
    problems.extend(check_aa("present/passes/smaa.rs", "record_smaa", "Smaa", &non_test_lines(&read("present/passes/smaa.rs"))));
    problems.extend(check_cold_bodies(&read("present/gpu_crumbs.rs")));

    let mut distinct = names.clone();
    distinct.sort();
    distinct.dedup();
    println!(
        "[G1] literal add_pass sites {} ({} distinct names); guarded GB call sites {}; funnels {}; recorders {}",
        names.len(),
        distinct.len(),
        guarded_sites,
        FUNNELS.len(),
        RECORDERS.len()
    );
    // (f) Non-vacuity: 3 funnels + 3 frame closes + 3 present blits + FXAA + SMAA + begin + verify.
    if names.len() < MIN_LITERAL_PASS_NAMES {
        problems.push(format!(
            "the census saw {} literal pass names, fewer than the {MIN_LITERAL_PASS_NAMES} at its rung: the scanner went blind",
            names.len()
        ));
    }
    if guarded_sites < 13 {
        problems.push(format!("the census saw {guarded_sites} GB call sites, fewer than 13: a site was lost"));
    }
    assert!(problems.is_empty(), "G1 found {} problem(s):\n{}", problems.len(), problems.join("\n"));
}

/// The scanner's own red: each malformed shape is refused, so a green above means something.
#[test]
fn the_census_refuses_what_it_exists_to_refuse() {
    let file = "fixture.rs";
    for bad in [r#"        g.add_pass("");"#, r#"        g.add_pass("Bad Name");"#, "        g.add_pass(name);"] {
        let lines = vec![bad.to_string()];
        let (_, problems) = check_pass_names(file, &lines, bad);
        assert_eq!(problems.len(), 1, "`{bad}` must be refused: {problems:?}");
    }
    let good = r#"        g.add_pass("ssao_atrous");"#;
    let (n, p) = check_pass_names(file, &[good.to_string()], good);
    assert!(p.is_empty() && n == ["ssao_atrous"], "{p:?} {n:?}");
    let array_text = "const NAMES: [&str; 2] = [\"a_0\", \"Bad\"];\n        g.add_pass(NAMES[p]);";
    let (_, p) = check_pass_names(file, &non_test_lines(array_text), array_text);
    assert_eq!(p.len(), 1, "a const array holding a bad name must be refused: {p:?}");

    // A funnel with its `crumb_enter_pass` removed.
    let funnel = [
        "    pub(crate) fn record_vb_pass(&self, pass: PassId, cmd: VkCommandBuffer) {",
        "        let mut sink = VbBarrierSink { fns: self.fns, cmd };",
        "        self.frame_graph.record_pass(pass, &mut sink);",
        "    }",
    ]
    .map(String::from)
    .to_vec();
    let problems = check_funnels(&funnel);
    assert!(
        problems.iter().any(|p| p == "record_vb_pass: 0 crumb_enter_pass, want 1"),
        "a funnel without its scope must be named: {problems:?}"
    );

    // An unguarded site.
    let unguarded = ["        let x = 1;", "        self.crumb_frame_close(cmd);"].map(String::from).to_vec();
    let (sites, problems) = check_guards(file, &unguarded);
    assert_eq!((sites, problems.len()), (1, 1), "{problems:?}");
}
