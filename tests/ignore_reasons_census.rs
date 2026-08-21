//! **A bare `#[ignore]` is the third way to make a check disappear, and it is the only one this
//! repository never asked you to justify.**
//!
//! The other two are already governed, and by the same rule in both cases — *write down why*:
//!
//! * `unsafe` carries a mandatory `// SAFETY:` comment stating the invariants (CLAUDE.md,
//!   "Required for every `unsafe`"). One grep enumerates every block and its argument.
//! * `#[allow(clippy::disallowed_types)]` carries a mandatory rationale comment (CLAUDE.md,
//!   "Forbidden on the hot path"). One grep enumerates every exception and its argument.
//!
//! `#[ignore]` removes a test from every run of `cargo test` just as completely as `unsafe`
//! removes the borrow checker and `#[allow]` removes a lint — and, unlike those two, it removes it
//! *silently*: libtest prints `ignored` in a summary line nobody reads and the suite still says
//! `test result: ok`. A bare one leaves no record of what it was waiting for, so the only way to
//! find out whether it can be run today is to run it and see what breaks. That is the same failure
//! shape this corpus keeps finding under a different name — a check that cannot fail, discovered
//! only by the person who eventually needs it.
//!
//! This gate applies the existing rule to the third case. It does not judge whether an ignore is
//! *justified*; it requires that the justification be **written at the site**, exactly as
//! `// SAFETY:` and the `disallowed_types` rationale are.
//!
//! # What it asserts
//!
//! 1. **No attribute-position `#[ignore]` is bare.** `#[ignore]` and `#[cfg_attr(<cfg>, ignore)]`
//!    both fail; `#[ignore = "…"]` and `#[cfg_attr(<cfg>, ignore = "…")]` pass.
//! 2. **No reason is the empty string.** `#[ignore = ""]` satisfies the letter of the rule and
//!    none of its point, and would otherwise be the obvious way past this gate.
//! 3. **Every site resolves to a test function name.** The failure message has to name the *test*,
//!    not just the file — several files here carry twenty-seven ignores. Resolving the name for
//!    every site on every run (not only for violating ones) is deliberate: a reporter exercised
//!    only on the day it is needed is a dead datum, which is this corpus's most-repeated defect
//!    class. If the resolver breaks, the **green** run fails, not the red one.
//! 4. **The walk is not vacuous.** Floors on files visited and sites found, plus a requirement
//!    that BOTH attribute forms and at least three distinct crates appear — so a `SKIP_DIRS` typo
//!    that skipped `crates/`, or a detector that lost the `cfg_attr` shape, reds instead of
//!    reporting a triumphant zero over an empty set.
//! 5. **The waiver list is not stale.** [`BARE_IGNORE_WAIVERS`] is the escape hatch, in the shape
//!    `engine_packages_census.rs`'s `USER_PACKAGES` established: an explicit const, one row per
//!    entry, each row carrying its own reason. A row whose site is no longer bare must be deleted,
//!    or the list reads as coverage it no longer has.
//!
//! **It is empty today.** The 19 sites that were bare when this gate was written were annotated
//! rather than waived, because in every case the requirement was recoverable from the test body or
//! its module doc. A waiver is for the case where it is not — and it costs one line and a written
//! argument, which is the entire mechanism.
//!
//! # Why this lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree — `internal_docs_anchors.rs`'s rationale, verbatim. The package also has
//! effectively no dependencies, so the gate needs no GPU, no `dxc` and no golden corpus.
//!
//! # What it cannot claim
//!
//! It reads lines, not tokens. A `#[ignore]` written at the start of a line **inside a `/* … */`
//! block comment** would be reported as live code. That shape does not occur — the whole tree
//! contains one line-initial `/*`, measured — and every mention of `#[ignore]` outside attribute
//! position is a `//` comment, also measured. The trade is deliberate: a half-correct lexer that
//! is wrong in a way nobody predicted is worse than a line reader whose one blind spot is written
//! down here, and the failure mode is a *false red* naming a specific line, which takes seconds to
//! diagnose — not a false green.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Sites permitted to carry a bare `#[ignore]`, as `(file, test fn, why no reason can be given)`.
///
/// A row is a written argument that the requirement genuinely cannot be stated — not a parking
/// space for one nobody has worked out yet. If the requirement is merely *unknown*, the honest
/// reason string says so (`"unknown: last green on …, requirement not reconstructed"`), which is
/// still a reason and still readable by the recipe partition below; a waiver is for when even that
/// is false.
///
/// Empty by construction, and that is the finding worth keeping: the annotation pass that preceded
/// this gate reached all 19 bare sites without needing the hatch once.
const BARE_IGNORE_WAIVERS: &[(&str, &str, &str)] = &[
    // (No entries. Add one only with an argument, and delete it the moment the site gains a
    // reason — `no_waiver_row_is_stale` enforces the second half.)
];

/// Directories the walk skips outright.
///
/// `.claude` is here because agent sessions park REGISTERED WORKTREES of other branches under
/// `.claude/worktrees/` (gitignored). A census that walks them enumerates another checkout's
/// source as if it were this tree's, so the gate reds for whoever happens to have a worktree
/// parked — the same leak class as the recorded `clippy.toml` ancestor-walk hazard, and it was
/// MEASURED in `gpu_blocking_reader_census.rs`: two parked worktrees turned that census red in the
/// main checkout while a clean worktree at the same HEAD passed.
const SKIP_DIRS: &[&str] = &["target", ".git", ".claude", "graphify-out", "book", "assets"];

/// Floor on `.rs` files visited. The tree holds ~1500; this exists only to catch a walker that
/// stopped walking, not to track the count.
const MIN_FILES: usize = 800;

/// Floor on ignore sites found. The tree holds 164 (143 plain + 21 `cfg_attr`); this is well below
/// it and exists only to catch a detector that stopped detecting.
const MIN_SITES: usize = 120;

/// How far past the attribute to look for the `fn` it decorates. Real sites are 1–6 lines away
/// (`#[test]`, `#[should_panic]`, `#[allow(…)]` and doc comments may intervene); the cap keeps a
/// malformed file from running the resolver to the end of a 7000-line test module.
const FN_LOOKAHEAD: usize = 24;

/// What one source line says about `#[ignore]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoreForm {
    /// Not an ignore attribute — ordinary code, or prose quoting one.
    NotAnIgnore,
    /// `#[ignore = "…"]` — the shape this gate exists to require.
    Reasoned,
    /// `#[ignore]` — the check disappears and says nothing about what it was waiting for.
    Bare,
    /// `#[ignore = ""]` — the letter of the rule, none of its point.
    EmptyReason,
}

/// Which spelling the site used. Both remove the test; they are reported separately only so a
/// detector that silently lost one whole form fails the non-vacuity clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoreSpelling {
    /// `#[ignore …]`, unconditional.
    Plain,
    /// `#[cfg_attr(<cfg>, ignore …)]`, conditional on a cfg (in this tree, always `miri`).
    CfgAttr,
}

/// One `#[ignore]` in the tree.
struct Site {
    /// Repo-relative, `/`-separated.
    file: String,
    /// 1-indexed line of the attribute.
    line: usize,
    /// The test function the attribute decorates, or `None` if the resolver could not find one.
    test_fn: Option<String>,
    form: IgnoreForm,
    spelling: IgnoreSpelling,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// True for the identifier characters that make `ignore` part of a longer word.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Classify what follows the `ignore` token: `]` / `,` / `)` mean bare, `=` means a reason
/// follows, anything else means the token was not the attribute after all.
fn form_after_ignore_token(rest: &str) -> IgnoreForm {
    let rest = rest.trim_start();
    let mut chars = rest.chars();
    match chars.next() {
        Some(']' | ',' | ')') => IgnoreForm::Bare,
        Some('=') => {
            let value = rest[1..].trim_start();
            // `""` and `r""` are reasons in form only. Longer strings start `"x` / `r"x`.
            if value.starts_with("\"\"") || value.starts_with("r\"\"") {
                IgnoreForm::EmptyReason
            } else {
                IgnoreForm::Reasoned
            }
        }
        _ => IgnoreForm::NotAnIgnore,
    }
}

/// The first whole-token occurrence of `ignore` in `line`, returning what follows it.
///
/// Whole-token matching is what keeps `#[cfg_attr(feature = "ignore_slow", ignore)]` from being
/// read at its first hit: `ignore_slow` is followed by an identifier character, so it is skipped
/// and the real attribute is found.
fn after_ignore_token(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut from = 0usize;
    while let Some(offset) = line[from..].find("ignore") {
        let start = from + offset;
        let end = start + "ignore".len();
        let before_ok = start == 0 || !is_ident_char(bytes[start - 1] as char);
        let after_ok = line[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            return Some(&line[end..]);
        }
        from = end;
    }
    None
}

/// Square-bracket depth of `line`, counting only brackets OUTSIDE string literals — a reason
/// string is free to contain `[` (several real ones quote code), and counting those would leave
/// the join running past the attribute or stopping short of it.
fn bracket_depth(line: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// How many continuation lines an attribute may span. The longest real site is 8 lines; the cap
/// exists so a malformed file cannot make the join quadratic.
const ATTR_JOIN_LOOKAHEAD: usize = 12;

/// The attribute OPENING at `lines[index]`, joined across continuation lines until its square
/// brackets balance. A single-line attribute returns itself unchanged.
///
/// This exists because the classifier's first shipped form was line-at-a-time, and a verification
/// pass PROVED the hole by construction: a bare multi-line `#[cfg_attr(…, ignore)]` split across
/// four lines was walked (the file count moved) and not counted (the site count did not). Two
/// live reasoned
/// sites had the same shape — one of them created by the very annotation pass this gate ships
/// with, which reformatted a long reason onto continuation lines and thereby moved it out of the
/// gate's own field of view.
fn joined_attribute(lines: &[&str], index: usize) -> String {
    let first = lines[index].trim_start();
    let mut depth = bracket_depth(first);
    let mut joined = first.to_string();
    if !first.starts_with("#[") || depth <= 0 {
        return joined;
    }
    for cont in lines.iter().skip(index + 1).take(ATTR_JOIN_LOOKAHEAD) {
        joined.push(' ');
        joined.push_str(cont.trim());
        depth += bracket_depth(cont);
        if depth <= 0 {
            break;
        }
    }
    joined
}

/// The first whole occurrence of `#[ignore` in attribute text, returning what follows the token.
/// Searching the WHOLE text rather than the prefix is what catches `#[test] #[ignore]` on one
/// line — the second proven hole of the line-at-a-time form.
fn after_plain_ignore_attr(text: &str) -> Option<&str> {
    let pos = text.find("#[ignore")?;
    let rest = &text[pos + "#[ignore".len()..];
    // A longer identifier (`#[ignored_by…]`, hypothetically) is not the attribute.
    match rest.chars().next() {
        Some(c) if is_ident_char(c) => None,
        _ => Some(rest),
    }
}

/// Read one line as an ignore attribute, or not.
fn classify(line: &str) -> (IgnoreForm, IgnoreSpelling) {
    let trimmed = line.trim_start();

    // Prose. Every `#[ignore]` mention outside attribute position in this tree is a `//`, `///`
    // or `//!` comment (measured), and a doc comment quoting the attribute — of which there are
    // dozens, because the convention is documented at the sites that follow it — must not be read
    // as one. `*` catches the `/** … */` continuation style.
    if trimmed.starts_with("//") || trimmed.starts_with('*') {
        return (IgnoreForm::NotAnIgnore, IgnoreSpelling::Plain);
    }

    // Only attribute text is a candidate: string literals QUOTING an attribute (this file's own
    // classifier table is full of them) sit on lines that start with `(`/`"` and are excluded
    // here, exactly as prose is excluded above.
    if !trimmed.starts_with("#[") {
        return (IgnoreForm::NotAnIgnore, IgnoreSpelling::Plain);
    }
    if let Some(rest) = after_plain_ignore_attr(trimmed) {
        return (form_after_ignore_token(rest), IgnoreSpelling::Plain);
    }
    if let Some(pos) = trimmed.find("#[cfg_attr(")
        && let Some(rest) = after_ignore_token(&trimmed[pos..])
    {
        return (form_after_ignore_token(rest), IgnoreSpelling::CfgAttr);
    }
    (IgnoreForm::NotAnIgnore, IgnoreSpelling::Plain)
}

/// The name of the `fn` an attribute at `attr_index` decorates.
///
/// Skips the intervening attributes and doc comments rather than requiring the `fn` to be the very
/// next line, because `#[test] #[should_panic] #[ignore = "…"]` in either order is a real shape.
fn resolve_test_fn(lines: &[&str], attr_index: usize) -> Option<String> {
    for line in lines.iter().skip(attr_index + 1).take(FN_LOOKAHEAD) {
        let t = line.trim_start();
        // Doc comments sit between the attributes and the `fn` constantly here, and they talk
        // about functions; a prose `fn foo` in one would otherwise answer for the real signature.
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }
        // `pub fn` / `pub(crate) fn` / `async fn` / `unsafe fn` all end in `fn ` before the name.
        let Some(pos) = t.find("fn ") else { continue };
        // Only accept `fn` at a word boundary, so `// turn fn into …` prose cannot answer.
        if pos > 0 && is_ident_char(t.as_bytes()[pos - 1] as char) {
            continue;
        }
        let after = &t[pos + 3..];
        let name: String = after.chars().take_while(|c| is_ident_char(*c)).collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// Every `.rs` file under `dir`, repo-relative, `/`-separated.
fn collect_rs(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_rs(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

/// Every ignore site in the tree, plus the number of `.rs` files the walk visited.
fn census() -> (Vec<Site>, usize) {
    let root = repo_root();
    let mut files = Vec::new();
    collect_rs(&root, &root, &mut files);

    let mut sites = Vec::new();
    for rel in &files {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        for index in 0..lines.len() {
            // Continuation lines of a multi-line attribute never begin with `#[`, so joining at
            // every opener double-counts nothing.
            let joined = joined_attribute(&lines, index);
            let (form, spelling) = classify(&joined);
            if form == IgnoreForm::NotAnIgnore {
                continue;
            }
            sites.push(Site {
                file: rel.to_string_lossy().replace('\\', "/"),
                line: index + 1,
                test_fn: resolve_test_fn(&lines, index),
                form,
                spelling,
            });
        }
    }
    (sites, files.len())
}

/// The crate directory a repo-relative path belongs to, e.g. `crates/boyko_ecs`.
fn crate_of(file: &str) -> String {
    let mut parts = file.split('/');
    match (parts.next(), parts.next()) {
        (Some("crates"), Some(name)) => format!("crates/{name}"),
        _ => "<root>".to_string(),
    }
}

#[test]
fn a_multi_line_attribute_is_joined_before_classification() {
    // The bare multi-line form — the proven hole: walked, not counted, until the join landed.
    let bare: Vec<&str> = "#[cfg_attr(
    miri,
    ignore
)]".lines().collect();
    let joined = joined_attribute(&bare, 0);
    assert_eq!(
        classify(&joined),
        (IgnoreForm::Bare, IgnoreSpelling::CfgAttr),
        "a bare multi-line cfg_attr ignore must be SEEN, and seen as bare"
    );

    // The reasoned multi-line form — two live sites ship in this shape (reduce.rs, fixed_loop.rs).
    let reasoned: Vec<&str> =
        "#[cfg_attr(
    miri,
    ignore = \"wall-time; see M-P20-1\"
)]".lines().collect();
    let joined = joined_attribute(&reasoned, 0);
    assert_eq!(classify(&joined), (IgnoreForm::Reasoned, IgnoreSpelling::CfgAttr));

    // A reason string CONTAINING brackets must not derail the balance count.
    let bracketed: Vec<&str> =
        "#[cfg_attr(
    miri,
    ignore = \"asserts steps[0] == floor[1]\"
)]"
            .lines()
            .collect();
    let joined = joined_attribute(&bracketed, 0);
    assert_eq!(
        classify(&joined),
        (IgnoreForm::Reasoned, IgnoreSpelling::CfgAttr),
        "brackets inside the reason string are not attribute brackets"
    );

    // The same-line double attribute — the second proven hole.
    assert_eq!(
        classify("#[test] #[ignore]"),
        (IgnoreForm::Bare, IgnoreSpelling::Plain),
        "a same-line `#[test] #[ignore]` must be seen"
    );
    assert_eq!(classify("#[test] #[ignore = \"needs a device\"]"), (
        IgnoreForm::Reasoned,
        IgnoreSpelling::Plain
    ));

    // A single-line attribute passes through the join unchanged.
    let single: Vec<&str> = vec!["#[ignore = \"solo\"]"];
    assert_eq!(joined_attribute(&single, 0), "#[ignore = \"solo\"]");
}

#[test]
fn every_ignore_attribute_states_a_reason() {
    let (sites, file_count) = census();

    // ── Non-vacuity. A walk that found nothing would report a triumphant green over an empty
    // set — the shape this corpus refuses. These floors are well below the live counts and exist
    // only to catch an instrument that died.
    assert!(
        file_count > MIN_FILES,
        "the source walk visited only {file_count} .rs files — the walker is broken, not the tree"
    );
    assert!(
        sites.len() >= MIN_SITES,
        "the detector found only {} ignore sites — it is broken, not the tree (SKIP_DIRS typo? \
         classifier regression?)",
        sites.len()
    );

    let plain = sites.iter().filter(|s| s.spelling == IgnoreSpelling::Plain).count();
    let cfg_attr = sites.iter().filter(|s| s.spelling == IgnoreSpelling::CfgAttr).count();
    assert!(
        plain > 0 && cfg_attr > 0,
        "the detector found {plain} plain and {cfg_attr} cfg_attr sites — one whole attribute \
         form has gone invisible, which is a silent hole exactly the size of that form"
    );

    let crates: BTreeSet<String> = sites.iter().map(|s| crate_of(&s.file)).collect();
    assert!(
        crates.len() >= 3,
        "ignore sites were found in only {crates:?} — the walk is not reaching the crates tree"
    );

    // ── The reporter, proved live on the GREEN path. Resolving the test name only when something
    // is already broken would make it a dead datum: silently wrong for however long it takes for
    // the first violation to appear, and wrong exactly when the message matters most.
    let unnamed: Vec<String> =
        sites.iter().filter(|s| s.test_fn.is_none()).map(|s| format!("{}:{}", s.file, s.line)).collect();
    assert!(
        unnamed.is_empty(),
        "the test-name resolver found no `fn` within {FN_LOOKAHEAD} lines of these ignore sites: \
         {unnamed:?}. Either the resolver is broken — in which case every failure message this \
         gate can ever print is missing the name of the test it is about — or an `#[ignore]` is \
         attached to something that is not a function."
    );

    // Live figures, printed rather than written down, so `-- --nocapture` reports what the gate is
    // actually enforcing instead of a number someone recorded once and never re-derived.
    println!(
        "[ignore census] {} sites ({plain} plain, {cfg_attr} cfg_attr) across {} crates, \
         {file_count} .rs files walked, {} waivers",
        sites.len(),
        crates.len(),
        BARE_IGNORE_WAIVERS.len()
    );

    // ── The clause itself.
    let waived: BTreeSet<(&str, &str)> =
        BARE_IGNORE_WAIVERS.iter().map(|(file, test, _)| (*file, *test)).collect();

    let violations: Vec<String> = sites
        .iter()
        .filter(|s| matches!(s.form, IgnoreForm::Bare | IgnoreForm::EmptyReason))
        .filter(|s| {
            let name = s.test_fn.as_deref().unwrap_or("<unnamed>");
            !waived.contains(&(s.file.as_str(), name))
        })
        .map(|s| {
            let name = s.test_fn.as_deref().unwrap_or("<unnamed>");
            let what = match s.form {
                IgnoreForm::EmptyReason => "empty reason string",
                _ => "bare #[ignore]",
            };
            format!("{}:{} `{name}` ({what})", s.file, s.line)
        })
        .collect();

    assert!(
        violations.is_empty(),
        "these `#[ignore]`s do not say what they are waiting for:\n  {}\n\n\
         WHAT TO DO — write the requirement into the attribute:\n\
         \x20   #[ignore = \"needs a real windowed GPU device; the orchestrator runs it\"]\n\
         \x20   #[cfg_attr(miri, ignore = \"tractability: 100k rows through the apply window\")]\n\n\
         The reason must name what the test NEEDS (a GPU, a cargo feature, process isolation, \
         wall-clock budget) or what it is WAITING FOR (an unimplemented milestone) — not merely \
         that it is slow. `#[ignore]` removes the test from every `cargo test` run and prints a \
         summary line nobody reads, so the reason string is the only record that survives; \
         CLAUDE.md already requires a written rationale for the other two ways to make a check \
         disappear (`// SAFETY:` on every `unsafe`, a comment on every \
         `#[allow(clippy::disallowed_types)]`), and this is the third.\n\n\
         If the requirement genuinely cannot be stated, add a row to BARE_IGNORE_WAIVERS in \
         tests/ignore_reasons_census.rs with the argument for why — one line, reviewed like any \
         other.",
        violations.join("\n  ")
    );
}

/// A waiver whose site is no longer bare must be deleted.
///
/// Without this, the list keeps reading as coverage it no longer has — the `gone` clause that
/// `gpu_blocking_reader_census.rs` added after a pinned row outlived its subject by seven commits.
#[test]
fn no_waiver_row_is_stale() {
    let (sites, _) = census();
    let bare: BTreeSet<(String, String)> = sites
        .iter()
        .filter(|s| matches!(s.form, IgnoreForm::Bare | IgnoreForm::EmptyReason))
        .map(|s| (s.file.clone(), s.test_fn.clone().unwrap_or_else(|| "<unnamed>".to_string())))
        .collect();

    let stale: Vec<&(&str, &str, &str)> = BARE_IGNORE_WAIVERS
        .iter()
        .filter(|(file, test, _)| !bare.contains(&((*file).to_string(), (*test).to_string())))
        .collect();

    assert!(
        stale.is_empty(),
        "these BARE_IGNORE_WAIVERS rows name sites that are no longer bare (or no longer exist): \
         {stale:?}. Good news, but the list must shrink deliberately — delete the rows."
    );
}

/// The detector's own positive control: every shape it will meet, classified by hand and checked.
///
/// The floors in the main test catch a detector that found *nothing*. They cannot catch one that
/// reads `#[cfg_attr(miri, ignore)]` as reasoned — that failure is invisible from the outside and
/// looks exactly like a clean tree, which is the whole reason it needs its own table.
#[test]
fn the_detector_classifies_every_shape_it_will_meet() {
    use IgnoreForm::{Bare, EmptyReason, NotAnIgnore, Reasoned};
    use IgnoreSpelling::{CfgAttr, Plain};

    let cases: &[(&str, IgnoreForm, IgnoreSpelling)] = &[
        // The two shapes that must FAIL — the defect this gate exists for.
        ("#[ignore]", Bare, Plain),
        ("    #[ignore]", Bare, Plain),
        ("#[ignore ]", Bare, Plain),
        ("    #[cfg_attr(miri, ignore)]", Bare, CfgAttr),
        ("#[cfg_attr(all(miri, unix), ignore)]", Bare, CfgAttr),
        // The way past a naive presence check.
        ("#[ignore = \"\"]", EmptyReason, Plain),
        ("#[cfg_attr(miri, ignore = \"\")]", EmptyReason, CfgAttr),
        // The shapes that must PASS.
        ("#[ignore = \"needs a real windowed GPU device\"]", Reasoned, Plain),
        ("#[cfg_attr(miri, ignore = \"tractability: 100k rows\")]", Reasoned, CfgAttr),
        // A reason string continued onto the next line with a trailing `\` — 6 live sites do this.
        ("#[ignore = \"needs a real windowed GPU device (do NOT set \\", Reasoned, Plain),
        // Prose. Dozens of module and item docs quote the attribute at the sites that use it, so
        // reading them as code would red the tree on its own documentation.
        ("//! `#[ignore]`: needs a real windowed GPU device.", NotAnIgnore, Plain),
        ("/// `#[ignore]` because it asserts nothing.", NotAnIgnore, Plain),
        ("// The tests above are `#[cfg_attr(miri, ignore)]` because …", NotAnIgnore, Plain),
        ("     * `#[ignore]` in a block-comment continuation", NotAnIgnore, Plain),
        // Near misses that are not this attribute.
        ("#[ignored]", NotAnIgnore, Plain),
        ("#[ignore_slow]", NotAnIgnore, Plain),
        ("#[cfg_attr(feature = \"ignore_slow\", test)]", NotAnIgnore, Plain),
        ("#[test]", NotAnIgnore, Plain),
        ("let ignore = 3;", NotAnIgnore, Plain),
        // The whole-token rule, load-bearing: the first literal `ignore` here is inside a cfg
        // NAME, and a substring search would classify off it and call this reasoned.
        ("#[cfg_attr(feature = \"ignore_slow\", ignore)]", Bare, CfgAttr),
    ];

    for (line, want_form, want_spelling) in cases {
        let (form, spelling) = classify(line);
        assert_eq!(form, *want_form, "classify({line:?}) read the wrong form");
        if *want_form != NotAnIgnore {
            assert_eq!(spelling, *want_spelling, "classify({line:?}) read the wrong spelling");
        }
    }
}

/// The name resolver's own positive control, on the shapes it meets in this tree.
#[test]
fn the_test_name_resolver_finds_the_function_under_the_attributes() {
    let lines: Vec<&str> = vec![
        "#[test]",
        "#[ignore = \"needs a GPU\"]",
        "fn windowed_smoke_dumps_a_frame() {",
        "",
        "/// Doc between the attribute and the fn.",
        "#[test]",
        "#[should_panic(expected = \"invariant\")]",
        "#[cfg_attr(miri, ignore = \"tractability\")]",
        "#[allow(clippy::disallowed_types)]",
        "/// still doc",
        "pub(crate) async fn pool_growth_under_apply_window() {",
    ];
    assert_eq!(
        resolve_test_fn(&lines, 1).as_deref(),
        Some("windowed_smoke_dumps_a_frame"),
        "the resolver missed the fn one line below the attribute"
    );
    assert_eq!(
        resolve_test_fn(&lines, 7).as_deref(),
        Some("pool_growth_under_apply_window"),
        "the resolver missed a fn behind three intervening attributes and a doc comment"
    );

    // And it must report failure rather than inventing a name, because `every_ignore_attribute_\
    // states_a_reason` treats `None` as a broken reporter and reds on it.
    let orphan: Vec<&str> = vec!["#[ignore = \"x\"]", "const NOT_A_FN: u8 = 0;"];
    assert_eq!(
        resolve_test_fn(&orphan, 0),
        None,
        "the resolver named something that is not a function"
    );
}
