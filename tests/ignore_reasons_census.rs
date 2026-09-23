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
//! 6. **Every reason starts with a class from a closed vocabulary** — `<class>: <prose>`, the
//!    classes in [`IGNORE_CLASSES`] (CLAUDE.md, "The ignored suite"). A missing prefix is RED, a
//!    prefix outside the list is RED, and so is a class the site's own `cfg` contradicts. See
//!    "The class prefix" below; `every_site_carries_a_class_its_predicate_allows` is the clause.
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
//! It reads lines, not tokens — with one exception a measurement forced on it.
//! [`lines_beginning_in_a_string`] walks each file once and records, per line, whether that line
//! BEGINS inside a string literal; those lines are skipped before classification.
//!
//! **The argument that stood here instead was false.** It claimed that every mention of
//! `#[ignore]` outside attribute position is a `//` comment, and the classifier said the same of
//! string literals quoting an attribute — that they "sit on lines that start with `(`/`\"`" and
//! are therefore excluded by the `#[` prefix test. Two live lines disprove both:
//! `crates/boyko_threadpool/tests/a6_panicked_scope_chunk_receipts.rs:119` and
//! `crates/boyko_threadpool/tests/block_allocation_receipts.rs:459` are continuation lines of a
//! `\`-continued reason string inside a `panic!`, and their first non-whitespace characters are
//! `#[cfg_attr(`. Both were counted as real sites, so the published number was **2 too high**. The
//! gate stayed green only by luck of shape: a phantom that reads as *reasoned* is merely a wrong
//! count, while one shaped like a bare `#[ignore]` is a FALSE RED naming a line inside a string
//! literal — a failure whose stated cause is not where the reader must look.
//!
//! The scanner therefore tracks what it takes to answer that one question honestly: escapable,
//! raw (any hash count) and `b`/`c`-prefixed strings, char literals (`'"'` must not open one, and
//! a lifetime `&'a T` must not read as an unterminated one), `//` comments, and `/* … */`
//! comments, which nest in Rust.
//!
//! What is still outside its field of view is the block comment: a `#[ignore]` at the start of a
//! line inside `/* … */` is reported as live code. The scanner does know it is in a comment there
//! — it must, or a quote inside one would desynchronise the string tracking — and the skip is
//! deliberately not extended to it, because that shape is not this tree's: the whole tree contains
//! one line-initial `/*` (measured; inside a raw-string fixture in `boyko_log`'s
//! `code_registry.rs`), commenting an attribute out leaves a `//` at the line start, and the
//! failure mode there is a *false red* naming a specific line, which takes seconds to diagnose —
//! not a false green.
//!
//! # The class prefix (UG-11, rung B3, 2026-09-23)
//!
//! A reason says why; the prefix says **which leg runs the test**, so that every leg is a `grep`
//! rather than a reading. CLAUDE.md records why the partition cannot be derived from prose: a
//! keyword classifier put 10 of 143 plain sites on the device-free side and 8 of the 10 were
//! wrong, all toward green. So every site was classified once, by reading what the test needs,
//! and this gate keeps the result from rotting.
//!
//! * **Grammar.** `[feature+]<class>: <prose>`. The only multi-class spellings are
//!   `feature+<class>` (the test does not exist or does not run without a cargo feature, and needs
//!   `<class>` besides) and `gpu-windowed+gpu-cap`, so one requirement set has one spelling and
//!   one grep. `generator`, `deferred` and `flaky` stand alone: they name no leg, and no leg may
//!   sweep them in.
//! * **Scope.** A `cfg_attr` site is ignored only where its predicate holds, so the predicate
//!   limits the class. It is evaluated in native debug, native release and Miri. A site ignored
//!   only under Miri runs natively, so its class says why Miri skips it: `miri-slow` or
//!   `miri-unsupported` and nothing else. A site ignored only in native release names no leg (it
//!   runs in every debug run and cannot pass in release), so it needs a reason but no class.
//! * **Consistency, not classification.** Four rules, each the negation of a misfile measured in
//!   this tree. They check a class against what the source says; they never choose one.
//!   1. A test whose body reaches a device — an `*_or_skip` helper, one of
//!      [`DEVICE_ENTRY_CALLS`], a same-file fn returning `Option<VulkanContext>`, or a same-file fn
//!      that reaches one of these ([`device_helpers`]) — carries a `gpu*` class (or a no-leg class).
//!      This is the `boot_*_or_skip` shape that returns early and PASSES on a box without a GPU.
//!   2. A test that exists only under Miri (a `cfg` false natively, true under Miri) carries a
//!      `miri-*` class, and a `miri-*` class needs `miri` somewhere in the site's `cfg` context.
//!      This is the `#![cfg(miri)]` shape that prints `running 0 tests` natively and exits 0.
//!   3. A test that a cargo feature conditions (a `cfg` over it, or its own `cfg_attr`, names a
//!      feature) carries `feature`, and its prose names `--features <that feature>`; `feature`
//!      on a site no feature conditions is RED. Every `--features <name>` in any reason must be
//!      declared in the crate's `[features]` table, so a renamed feature reds.
//!   4. The scope rule above.
//!
//! What the rules cannot see: a device reached only through a child process (a driver that
//! re-executes its own binary to run a windowed worker), through another module's helper
//! (`particle_scene::build_app`), or through a probe type with its own `open`; and a device-free
//! class chosen wrongly between `solo` and `slow`. Those were settled by reading; the per-site
//! record is the B3 migration's receipt, not this file.

use std::collections::{BTreeMap, BTreeSet};
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

/// Floor on ignore sites found. The tree held 328 (178 plain + 150 `cfg_attr`) when measured on
/// 2026-09-19 on the parallel-narrowphase lane after its L2 calibration; this is well below it and
/// exists only to catch a detector that stopped detecting.
/// The number above is a snapshot that moves with every campaign (it fell by one when A7 resolved
/// its red-first tests, after growing for weeks, then rose by ten with the boot-validation lane's
/// device gates, by one with the colored-solver-default lane's release-only SIMD on/off test and
/// by seven with the L2 calibration's `miri-slow` broadphase-policy tests, and by one with
/// L5 C3's `slow:` Jolt-pyramid arm of `narrowphase_parallel_equivalence.rs`, to 329 / 179)
/// — do NOT read it as the current count, and
/// do not tune this floor to it. `every_ignore_attribute_states_a_reason` prints the live figure on
/// every run (`-- --nocapture`), which is the only figure a reader should quote. Because this is a
/// floor, a prose count that drifts upward — here or in `CLAUDE.md` — stays green forever; both
/// have already done so twice (232 was right at `d552be05` and wrong 48 sites later, in the same
/// lane that edited this file; the 280 this line carried before was what the gate printed on
/// 2026-09-17 — one of them a phantom the string scanner now rejects — and it was 31 short by the
/// time the A6 lane's three new test files were merged in).
const MIN_SITES: usize = 120;

/// How far past the attribute to look for the `fn` it decorates. Real sites are 1–6 lines away
/// (`#[test]`, `#[should_panic]`, `#[allow(…)]` and doc comments may intervene); the cap keeps a
/// malformed file from running the resolver to the end of a 7000-line test module.
const FN_LOOKAHEAD: usize = 24;

/// The closed reason-prefix vocabulary, as `(class, the leg that runs a site carrying it)`.
///
/// The list is CLAUDE.md's ("The ignored suite — legs by what the machine has") and it does not
/// grow here: the B3 migration re-prefixed the five out-of-list prefixes it found (`tractability`,
/// `instrument`, `M2`, `calibration`, `miri-arm`) rather than admitting them.
const IGNORE_CLASSES: [(&str, &str); 11] = [
    ("gpu", "device leg, headless: a Vulkan device and no window; per binary, `-- --ignored --test-threads=1`"),
    ("gpu-windowed", "device leg on a desktop session: a device, a window and a swapchain"),
    (
        "gpu-cap",
        "device leg on hardware with the optional capability the prose names (RT, ray query, \
         VK_KHR_pipeline_executable_properties); spelled `gpu-windowed+gpu-cap` when a window is \
         needed too",
    ),
    (
        "feature",
        "exists or runs only with `--features <name>`, which the prose names; `feature+<class>` \
         names the rest of the leg",
    ),
    ("solo", "device-free `-- --ignored` leg with `--test-threads=1`: the test holds process-wide state"),
    (
        "slow",
        "device-free, wall-clock budget: a plain site runs in the `-- --ignored` leg; a site ignored \
         in debug but not in release runs in the ordinary `--release` run",
    ),
    (
        "miri-slow",
        "runs natively; the Miri leg skips it because Miri would finish it only given hours (a test \
         that exists only under Miri runs in `cargo miri test … -- --ignored`)",
    ),
    (
        "miri-unsupported",
        "runs natively; Miri cannot execute what it needs at all (a child process, a custom \
         `#[global_allocator]`, a deliberate leak)",
    ),
    (
        "generator",
        "no leg: asserts nothing beyond an instrument sanity check and emits an artifact; run it by \
         name when the artifact is wanted",
    ),
    ("deferred", "no leg: red by design until a named decision or milestone lands"),
    ("flaky", "no leg: nondeterministic by design"),
];

/// Classes that name no leg, so they may not be combined with one.
const STANDALONE_CLASSES: [&str; 3] = ["generator", "deferred", "flaky"];

/// Classes that put a test on a GPU.
const DEVICE_CLASSES: [&str; 3] = ["gpu", "gpu-windowed", "gpu-cap"];

/// Classes that explain why the Miri leg skips a test.
const MIRI_CLASSES: [&str; 2] = ["miri-slow", "miri-unsupported"];

/// Every class set a reason may name after an optional leading `feature`, in its one spelling.
/// A single class is always a valid spelling on its own; this list is what may follow `feature+`
/// and the one multi-class spelling without it.
const CLASS_TAILS: [&[&str]; 8] = [
    &["gpu"],
    &["gpu-windowed"],
    &["gpu-cap"],
    &["gpu-windowed", "gpu-cap"],
    &["solo"],
    &["slow"],
    &["miri-slow"],
    &["miri-unsupported"],
];

/// Calls that put the calling test on a device, beside `*_or_skip` helpers and same-file fns
/// returning `Option<VulkanContext>`. MEASURED on this tree (2026-09-23): the headless tests boot
/// through `VulkanContext::boot`, the `boyko_app` tests through `EnginePlugins::window`, and
/// `window_present_gbuffer.rs` through its `with_windowed_present`.
const DEVICE_ENTRY_CALLS: [&str; 3] = ["VulkanContext::boot", "EnginePlugins::window", "with_windowed_present"];

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
    /// `#[cfg_attr(<cfg>, ignore …)]`, conditional on a cfg (in this tree, usually `miri`).
    CfgAttr,
}

/// One `#[ignore]` in the tree.
struct Site {
    /// Repo-relative, `/`-separated.
    file: String,
    /// 1-indexed line of the attribute.
    line: usize,
    /// Byte offset of the start of that line in the file's text.
    offset: usize,
    /// The test function the attribute decorates, or `None` if the resolver could not find one.
    test_fn: Option<String>,
    form: IgnoreForm,
    spelling: IgnoreSpelling,
    /// The reason string as rustc reads it (escapes and `\`-continuations decoded), or `None`
    /// when the attribute has none or the literal could not be read.
    reason: Option<String>,
    /// The predicate of a `#[cfg_attr(<predicate>, ignore …)]`, as written; `None` for a plain
    /// site, or when the predicate could not be delimited.
    cfg: Option<String>,
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

    // Only attribute text is a candidate. This test is NOT what excludes string literals quoting
    // an attribute: a `\`-continued reason string inside a `panic!` puts `#[cfg_attr(` at the
    // start of its continuation line and walks straight through here — two live ones did, and
    // were counted. `lines_beginning_in_a_string` is what excludes them, before this is reached.
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

/// The lexer state the per-line scanner carries across a newline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// Ordinary code.
    Code,
    /// Inside `// …`, which ends at the newline.
    LineComment,
    /// Inside `/* … */`, which NESTS in Rust; the payload is the nesting depth.
    BlockComment(u32),
    /// Inside a string literal. `Some(n)` is a raw string closed by `"` followed by `n` `#`;
    /// `None` is an escapable one (`"…"`, `b"…"`, `c"…"`), where `\"` does not close it.
    Str(Option<usize>),
}

/// A prefixed string literal opening at `chars[i]` — `b"`, `c"`, `r"`, `r#…"`, `br#…"`, `cr#…"` —
/// as `(characters consumed, hash count if raw)`.
///
/// The identifier-boundary test is load-bearing twice: the `r` of `for` must not open a literal,
/// and a raw identifier (`r#type`, whose `#` run is followed by a letter rather than a quote) is
/// rejected by the quote test below.
fn string_open_at(chars: &[char], i: usize) -> Option<(usize, Option<usize>)> {
    if i > 0 && is_ident_char(chars[i - 1]) {
        return None;
    }
    let mut j = i;
    if matches!(chars.get(j), Some('b' | 'c')) {
        j += 1;
    }
    let raw = chars.get(j) == Some(&'r');
    let mut hashes = 0usize;
    if raw {
        j += 1;
        while chars.get(j) == Some(&'#') {
            hashes += 1;
            j += 1;
        }
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    Some((j + 1 - i, if raw { Some(hashes) } else { None }))
}

/// How many characters the char literal at `chars[i]` occupies, or `1` if that `'` opens a
/// lifetime or a loop label instead.
///
/// `'"'` is the case that matters here: read as a lifetime, its quote would open a string literal
/// that never closes, and every line after it in the file would be reported as living inside one.
fn char_literal_len(chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        // `'\''`, `'\\'`, `'\u{10FFFF}'` — the escaped character sits at `i + 2`, so the earliest
        // closing quote is at `i + 3`; the cap covers the longest escape Rust spells.
        Some('\\') => {
            let limit = (i + 12).min(chars.len());
            for (offset, c) in chars.iter().enumerate().take(limit).skip(i + 3) {
                if *c == '\'' {
                    return offset + 1 - i;
                }
            }
            1
        }
        // `'x'`, including `'"'` and any multi-byte char.
        Some(_) if chars.get(i + 2) == Some(&'\'') => 3,
        // `&'a T`, `'outer: loop` — not a literal at all.
        _ => 1,
    }
}

/// What one character of a source file is, as far as this census needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lex {
    /// Code, including the delimiters of a literal (`"`, `r#"`, `"#`, the quotes of `'x'`).
    Code,
    /// Inside a `//` or `/* … */` comment, delimiters included.
    Comment,
    /// The body of a string or char literal.
    Literal,
}

/// Every character of `chars` lexed, one entry per character.
///
/// One scanner answers both questions this file asks of a lexer — whether a line BEGINS inside a
/// string ([`lines_beginning_in_a_string`]) and which bytes are code ([`code_mask`]) — so the two
/// cannot disagree about where a literal ends. A newline is classified by the state it is read
/// in: inside a string it is literal text, and a `//` comment ends at it.
fn lex(chars: &[char]) -> Vec<Lex> {
    fn emit(out: &mut Vec<Lex>, lex: Lex, n: usize) {
        out.extend(std::iter::repeat_n(lex, n));
    }
    let mut out = Vec::with_capacity(chars.len());
    let mut state = Scan::Code;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            if state == Scan::LineComment {
                state = Scan::Code;
            }
            let lexed = match state {
                Scan::Str(_) => Lex::Literal,
                Scan::BlockComment(_) => Lex::Comment,
                Scan::Code | Scan::LineComment => Lex::Code,
            };
            emit(&mut out, lexed, 1);
            i += 1;
            continue;
        }
        match state {
            Scan::LineComment => {
                emit(&mut out, Lex::Comment, 1);
                i += 1;
            }
            Scan::BlockComment(depth) => {
                if c == '/' && next == Some('*') {
                    state = Scan::BlockComment(depth + 1);
                    emit(&mut out, Lex::Comment, 2);
                    i += 2;
                } else if c == '*' && next == Some('/') {
                    state = if depth == 1 { Scan::Code } else { Scan::BlockComment(depth - 1) };
                    emit(&mut out, Lex::Comment, 2);
                    i += 2;
                } else {
                    emit(&mut out, Lex::Comment, 1);
                    i += 1;
                }
            }
            Scan::Str(Some(hashes)) => {
                let closes = c == '"'
                    && chars.len() >= i + 1 + hashes
                    && chars[i + 1..i + 1 + hashes].iter().all(|h| *h == '#');
                if closes {
                    state = Scan::Code;
                    emit(&mut out, Lex::Code, 1 + hashes);
                    i += 1 + hashes;
                } else {
                    emit(&mut out, Lex::Literal, 1);
                    i += 1;
                }
            }
            Scan::Str(None) => {
                if c == '\\' {
                    // A `\`-continued literal escapes the NEWLINE itself, and the newline arm
                    // above must still see it, or every following line's flag shifts by one —
                    // which is precisely the shape at the two lines this scanner exists for.
                    let width = if next == Some('\n') { 1 } else { 2 };
                    emit(&mut out, Lex::Literal, width);
                    i += width;
                } else if c == '"' {
                    state = Scan::Code;
                    emit(&mut out, Lex::Code, 1);
                    i += 1;
                } else {
                    emit(&mut out, Lex::Literal, 1);
                    i += 1;
                }
            }
            Scan::Code => match c {
                '/' if next == Some('/') => {
                    state = Scan::LineComment;
                    emit(&mut out, Lex::Comment, 2);
                    i += 2;
                }
                '/' if next == Some('*') => {
                    state = Scan::BlockComment(1);
                    emit(&mut out, Lex::Comment, 2);
                    i += 2;
                }
                '"' => {
                    state = Scan::Str(None);
                    emit(&mut out, Lex::Code, 1);
                    i += 1;
                }
                '\'' => {
                    let len = char_literal_len(chars, i);
                    if len == 1 {
                        emit(&mut out, Lex::Code, 1);
                    } else {
                        emit(&mut out, Lex::Code, 1);
                        emit(&mut out, Lex::Literal, len - 2);
                        emit(&mut out, Lex::Code, 1);
                    }
                    i += len;
                }
                'r' | 'b' | 'c' => match string_open_at(chars, i) {
                    Some((len, hashes)) => {
                        state = Scan::Str(hashes);
                        emit(&mut out, Lex::Code, len);
                        i += len;
                    }
                    None => {
                        emit(&mut out, Lex::Code, 1);
                        i += 1;
                    }
                },
                _ => {
                    emit(&mut out, Lex::Code, 1);
                    i += 1;
                }
            },
        }
    }
    // A two-character step at the very end of the input (`\` as the last character of an
    // unterminated literal) emits one entry past it.
    out.truncate(chars.len());
    out
}

/// For every line of `text`, whether that line BEGINS inside a string literal.
///
/// The classifier reads line starts, so this one bit is all it needs from a lexer — but producing
/// it honestly takes a real scan: a quote inside a `//` comment, inside a `/* … */` comment, or
/// inside a raw string's body opens nothing. Index `i` of the result is the flag for line `i` of
/// `text.lines()`; a file ending in a newline yields one extra trailing entry, which no caller
/// indexes.
fn lines_beginning_in_a_string(text: &str) -> Vec<bool> {
    let chars: Vec<char> = text.chars().collect();
    let lexed = lex(&chars);
    // The first line begins in code by definition; every other line begins in whatever the
    // newline before it was read in.
    let mut flags = vec![false];
    flags.extend(chars.iter().zip(&lexed).filter(|(c, _)| **c == '\n').map(|(_, l)| *l == Lex::Literal));
    flags
}

/// `text` with every comment and every literal body blanked to spaces, byte for byte.
///
/// Byte offsets are preserved (a multi-byte character becomes as many spaces as it had bytes), so
/// a position found in the mask indexes the same place in `text`, and a position inside code is a
/// character boundary in both. Newlines are kept, so line numbers are too.
fn code_mask(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lexed = lex(&chars);
    let mut out = String::with_capacity(text.len());
    for (c, l) in chars.iter().zip(&lexed) {
        if *l == Lex::Code || *c == '\n' {
            out.push(*c);
        } else {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
    }
    out
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

/// Every ignore site in one file's text, in line order.
///
/// Split out of [`census`] so a fixture can be run through the SAME scanner, join and classifier
/// the walk uses, rather than through a re-implementation of them inside a test — the shape that
/// lets an instrument's test pass while the instrument is wrong.
fn sites_in_text(file: &str, text: &str) -> Vec<Site> {
    let lines: Vec<&str> = text.lines().collect();
    let begins_in_string = lines_beginning_in_a_string(text);
    // `text.lines()` splits after every `\n` (a `\r` before it stays in the line's bytes), so line
    // `k` starts right after the `k`-th newline.
    let line_starts: Vec<usize> =
        std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let mut sites = Vec::new();
    for (index, &offset) in line_starts.iter().enumerate().take(lines.len()) {
        // A line whose first non-whitespace characters are `#[cfg_attr(` may be the continuation
        // of a `\`-continued reason string inside a `panic!` rather than an attribute; two live
        // ones are, and were counted until this skip existed.
        if begins_in_string.get(index).copied().unwrap_or(false) {
            continue;
        }
        // Continuation lines of a multi-line attribute never begin with `#[`, so joining at
        // every opener double-counts nothing.
        let joined = joined_attribute(&lines, index);
        let (form, spelling) = classify(&joined);
        if form == IgnoreForm::NotAnIgnore {
            continue;
        }
        sites.push(Site {
            file: file.to_string(),
            line: index + 1,
            offset,
            test_fn: resolve_test_fn(&lines, index),
            form,
            spelling,
            reason: None,
            cfg: None,
        });
    }
    // The reason and the predicate are read from the source text, not from the joined line: a
    // reason continued over several lines must decode exactly as rustc reads it, and a predicate
    // is delimited by code-level parentheses, which only the mask can tell from quoted ones.
    if !sites.is_empty() {
        let mask = code_mask(text);
        for site in &mut sites {
            if site.form == IgnoreForm::Reasoned {
                site.reason = reason_at(text, &mask, site.offset, site.spelling);
            }
            if site.spelling == IgnoreSpelling::CfgAttr {
                site.cfg = cfg_attr_predicate_at(text, &mask, site.offset);
            }
        }
    }
    sites
}

/// The byte offset in `mask` of the whole-token `ignore` inside the attribute opening at `at`.
fn ignore_token_at(mask: &str, at: usize, spelling: IgnoreSpelling) -> Option<usize> {
    match spelling {
        IgnoreSpelling::Plain => Some(at + mask[at..].find("#[ignore")? + "#[".len()),
        IgnoreSpelling::CfgAttr => {
            let open = at + mask[at..].find("#[cfg_attr(")?;
            let rest = after_ignore_token(&mask[open..])?;
            Some(mask.len() - rest.len() - "ignore".len())
        }
    }
}

/// The reason string of the ignore attribute opening at byte `at`, decoded as rustc would.
fn reason_at(text: &str, mask: &str, at: usize, spelling: IgnoreSpelling) -> Option<String> {
    let token = ignore_token_at(mask, at, spelling)?;
    let after = token + "ignore".len();
    let eq = after + mask[after..].find(|c: char| !c.is_whitespace())?;
    if mask.as_bytes()[eq] != b'=' {
        return None;
    }
    let lit = eq + 1 + mask[eq + 1..].find(|c: char| !c.is_whitespace())?;
    decode_string_literal(&text[lit..])
}

/// The value of the string literal at the start of `src`: `"…"` with its escapes, or `r#*"…"#*`.
///
/// Only what a reason needs is decoded exactly — `\`-newline continuations (which drop the newline
/// and the next line's leading whitespace), `\"`, `\\`, `\n`, `\t`, `\r`, `\0` and `\'`; any other
/// escape keeps its character, which cannot move a prefix or a feature name.
fn decode_string_literal(src: &str) -> Option<String> {
    if let Some(raw) = src.strip_prefix('r') {
        let hashes = raw.chars().take_while(|c| *c == '#').count();
        let body = raw[hashes..].strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        return Some(body[..body.find(&close)?].to_string());
    }
    let mut out = String::new();
    let mut chars = src.strip_prefix('"')?.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '\n' | '\r' => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                other => out.push(other),
            },
            _ => out.push(c),
        }
    }
    None
}

/// The predicate of the `#[cfg_attr(<predicate>, ignore …)]` opening at byte `at`: everything up
/// to the first comma at the attribute's own parenthesis depth.
fn cfg_attr_predicate_at(text: &str, mask: &str, at: usize) -> Option<String> {
    let start = at + mask[at..].find("#[cfg_attr(")? + "#[cfg_attr(".len();
    let mut depth = 0i32;
    for (offset, b) in mask.as_bytes()[start..].iter().enumerate() {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' if depth == 0 => return None,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => return Some(text[start..start + offset].trim().to_string()),
            _ => {}
        }
    }
    None
}

/// Every ignore site in the tree, plus the number of `.rs` files the walk visited.
fn census() -> (Vec<Site>, usize) {
    let root = repo_root();
    let mut files = Vec::new();
    collect_rs(&root, &root, &mut files);

    let mut sites = Vec::new();
    for rel in &files {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else { continue };
        sites.extend(sites_in_text(&rel.to_string_lossy().replace('\\', "/"), &text));
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

// ═══════════════════════════ the class prefix ═══════════════════════════════

/// Why a reason's prefix is not a valid class.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrefixError {
    /// The reason does not start with `<word>: ` — it names no class.
    Missing,
    /// A `+`-part of the prefix is not in [`IGNORE_CLASSES`] (matching is case-sensitive).
    Unknown(String),
    /// Every part is in the vocabulary, but the combination is not one of its spellings.
    Combination(String),
    /// A class and nothing after it.
    NoProse,
}

/// The classes a reason names, in order, and the prose after them.
fn parse_prefix(reason: &str) -> Result<(Vec<&'static str>, &str), PrefixError> {
    let (head, rest) = reason.split_once(':').ok_or(PrefixError::Missing)?;
    // A head with a space in it is prose that happens to contain a colon ("Miri wall-time: …"),
    // and `gpu :` is not the documented shape either; both read as no prefix at all.
    let head_is_a_word =
        !head.is_empty() && head.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '_'));
    if !head_is_a_word {
        return Err(PrefixError::Missing);
    }
    let mut classes = Vec::new();
    for part in head.split('+') {
        let known = IGNORE_CLASSES.iter().find(|(class, _)| *class == part);
        classes.push(known.ok_or_else(|| PrefixError::Unknown(part.to_string()))?.0);
    }
    let tail_is_spelled = |tail: &[&str]| CLASS_TAILS.contains(&tail);
    let spelled = match classes.as_slice() {
        [_] => true,
        ["feature", tail @ ..] => tail_is_spelled(tail),
        tail => tail_is_spelled(tail),
    };
    if !spelled {
        return Err(PrefixError::Combination(head.to_string()));
    }
    let prose = rest.trim();
    if prose.is_empty() {
        return Err(PrefixError::NoProse);
    }
    // `gpu:x` would parse, but the prefix is keyed on `<class>: ` by every grep that selects a leg.
    if !rest.starts_with(char::is_whitespace) {
        return Err(PrefixError::Missing);
    }
    Ok((classes, prose))
}

/// Every feature name a reason passes to `--features` (comma-separated lists split).
fn features_named(reason: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut from = 0usize;
    while let Some(offset) = reason[from..].find("--features") {
        let after = from + offset + "--features".len();
        let list = reason[after..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
        let token: String =
            list.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ',' | '/')).collect();
        out.extend(token.split(',').filter(|name| !name.is_empty()).map(str::to_string));
        from = after;
    }
    out
}

// ═══════════════════════════ cfg predicates ═════════════════════════════════

/// A `cfg` predicate, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pred {
    All(Vec<Pred>),
    Any(Vec<Pred>),
    Not(Box<Pred>),
    /// `miri`, `debug_assertions`, `test`, `windows`, …
    Name(String),
    /// `feature = "x"`, `target_os = "x"`, …
    Pair(String, String),
}

/// One configuration a predicate is evaluated in.
#[derive(Debug, Clone, Copy)]
struct CfgEnv {
    miri: bool,
    debug_assertions: bool,
    /// Whether every `feature = "…"` holds. The census asks two questions of a feature atom —
    /// "does the default build see this?" (all off) and "does some build?" (all on).
    features: bool,
}

/// `cargo test`: the configuration every rung's UG-01 runs.
const NATIVE_DEBUG: CfgEnv = CfgEnv { miri: false, debug_assertions: true, features: false };
/// `cargo test --release`.
const NATIVE_RELEASE: CfgEnv = CfgEnv { miri: false, debug_assertions: false, features: false };
/// `cargo miri test`, which builds with debug assertions.
const MIRI: CfgEnv = CfgEnv { miri: true, debug_assertions: true, features: false };

impl CfgEnv {
    const fn with_features(self, features: bool) -> Self {
        Self { features, ..self }
    }
}

/// Parse a predicate as written between `cfg(` and its `)`, or `None` if it is not one.
fn parse_pred(src: &str) -> Option<Pred> {
    #[derive(Debug, Clone, PartialEq)]
    enum Tok {
        Ident(String),
        Str(String),
        Open,
        Close,
        Comma,
        Eq,
    }
    let mut toks = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' | ')' | ',' | '=' => {
                chars.next();
                toks.push(match c {
                    '(' => Tok::Open,
                    ')' => Tok::Close,
                    ',' => Tok::Comma,
                    _ => Tok::Eq,
                });
            }
            '"' => {
                chars.next();
                let value: String = chars.by_ref().take_while(|c| *c != '"').collect();
                toks.push(Tok::Str(value));
            }
            c if is_ident_char(c) => {
                let mut ident = String::new();
                while let Some(&c) = chars.peek().filter(|c| is_ident_char(**c)) {
                    ident.push(c);
                    chars.next();
                }
                toks.push(Tok::Ident(ident));
            }
            _ => return None,
        }
    }

    fn one(toks: &[Tok], i: &mut usize) -> Option<Pred> {
        let Some(Tok::Ident(name)) = toks.get(*i).cloned() else { return None };
        *i += 1;
        match toks.get(*i) {
            Some(Tok::Eq) => {
                let Some(Tok::Str(value)) = toks.get(*i + 1).cloned() else { return None };
                *i += 2;
                Some(Pred::Pair(name, value))
            }
            Some(Tok::Open) => {
                *i += 1;
                let mut args = Vec::new();
                while toks.get(*i) != Some(&Tok::Close) {
                    args.push(one(toks, i)?);
                    match toks.get(*i) {
                        Some(Tok::Comma) => *i += 1,
                        Some(Tok::Close) => {}
                        _ => return None,
                    }
                }
                *i += 1;
                match name.as_str() {
                    "all" => Some(Pred::All(args)),
                    "any" => Some(Pred::Any(args)),
                    "not" if args.len() == 1 => Some(Pred::Not(Box::new(args.remove(0)))),
                    _ => None,
                }
            }
            _ => Some(Pred::Name(name)),
        }
    }

    let mut i = 0usize;
    let pred = one(&toks, &mut i)?;
    (i == toks.len()).then_some(pred)
}

impl Pred {
    /// Whether the predicate holds in `env`.
    ///
    /// `miri`, `debug_assertions` and `feature = "…"` follow `env`. EVERY OTHER atom (`test`,
    /// `windows`, `unix`, `target_os = "…"`, …) is taken to hold: the census decides scope along
    /// the Miri, profile and feature axes, and the platform axis is not one of them — a Windows-only
    /// file is on the Windows legs, which is what its class already says.
    fn holds(&self, env: CfgEnv) -> bool {
        match self {
            Pred::All(args) => args.iter().all(|p| p.holds(env)),
            Pred::Any(args) => args.iter().any(|p| p.holds(env)),
            Pred::Not(inner) => !inner.holds(env),
            Pred::Name(name) if name == "miri" => env.miri,
            Pred::Name(name) if name == "debug_assertions" => env.debug_assertions,
            Pred::Pair(key, _) if key == "feature" => env.features,
            Pred::Name(_) | Pred::Pair(..) => true,
        }
    }

    /// Whether the atom `name` appears anywhere in the predicate.
    fn mentions(&self, name: &str) -> bool {
        match self {
            Pred::All(args) | Pred::Any(args) => args.iter().any(|p| p.mentions(name)),
            Pred::Not(inner) => inner.mentions(name),
            Pred::Name(n) => n == name,
            Pred::Pair(..) => false,
        }
    }

    /// Every `feature = "…"` value in the predicate.
    fn features(&self, out: &mut BTreeSet<String>) {
        match self {
            Pred::All(args) | Pred::Any(args) => {
                for p in args {
                    p.features(out);
                }
            }
            Pred::Not(inner) => inner.features(out),
            Pred::Pair(key, value) if key == "feature" => {
                out.insert(value.clone());
            }
            Pred::Name(_) | Pred::Pair(..) => {}
        }
    }

    /// True if some Miri setting makes the predicate false with features off and true with them on.
    fn requires_a_feature(&self) -> bool {
        [NATIVE_DEBUG, MIRI].iter().any(|env| !self.holds(env.with_features(false)) && self.holds(env.with_features(true)))
    }

    /// True if the predicate is false in every native configuration and true in some Miri one.
    fn is_miri_only(&self) -> bool {
        let native = [NATIVE_DEBUG, NATIVE_RELEASE];
        let never_native = [false, true].iter().all(|f| native.iter().all(|env| !self.holds(env.with_features(*f))));
        never_native && [false, true].iter().any(|f| self.holds(MIRI.with_features(*f)))
    }
}

/// Where a site is ignored, which limits the class it may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    /// Ignored in native debug — every plain site, and `cfg_attr` sites whose predicate holds there.
    Native,
    /// Ignored in native release only: the test runs in every debug run.
    ReleaseOnly,
    /// Ignored under Miri only: the test runs natively.
    MiriOnly,
    /// Ignored only when some cargo feature is on.
    FeatureOnly,
}

impl Scope {
    fn of(pred: &Pred) -> Scope {
        if pred.holds(NATIVE_DEBUG) {
            Scope::Native
        } else if pred.holds(NATIVE_RELEASE) {
            Scope::ReleaseOnly
        } else if pred.holds(MIRI) {
            Scope::MiriOnly
        } else {
            Scope::FeatureOnly
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Scope::Native => "native",
            Scope::ReleaseOnly => "release-only",
            Scope::MiriOnly => "miri-only",
            Scope::FeatureOnly => "feature-only",
        }
    }
}

// ═══════════════════════════ what the source says about a site ══════════════

/// A `#[cfg(…)]` or `#![cfg(…)]` and the byte range of what it governs.
struct Gate {
    start: usize,
    end: usize,
    /// `Err` holds the predicate text when the parser could not read it.
    pred: Result<Pred, String>,
}

/// Every `{`/`}` pair in `mask`, as a map from each opening brace to its closing one.
fn brace_pairs(mask: &str) -> BTreeMap<usize, usize> {
    let mut pairs = BTreeMap::new();
    let mut open = Vec::new();
    for (i, b) in mask.bytes().enumerate() {
        match b {
            b'{' => open.push(i),
            b'}' => {
                if let Some(o) = open.pop() {
                    pairs.insert(o, i);
                }
            }
            _ => {}
        }
    }
    pairs
}

/// The index just past the `close` that balances the `open` at `from` (which must be `open`).
fn balanced_end(mask: &str, from: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, b) in mask.as_bytes()[from..].iter().enumerate() {
        if *b == open {
            depth += 1;
        } else if *b == close {
            depth -= 1;
            if depth == 0 {
                return Some(from + offset + 1);
            }
        }
    }
    None
}

/// Every `cfg` in a file and the range it governs.
///
/// An inner `#![cfg(…)]` governs the block it sits in (the whole file at brace depth 0). An outer
/// `#[cfg(…)]` governs the item after it: through the matching `}` of the first `{`, or to the
/// first `;`, `,` or unmatched `}` at parenthesis depth 0, whichever comes first — a fn or a mod,
/// a `use` or a `let`, a field or an argument.
fn gates_in(text: &str, mask: &str, braces: &BTreeMap<usize, usize>) -> Vec<Gate> {
    let bytes = mask.as_bytes();
    let mut gates = Vec::new();
    for (marker, inner) in [("#![cfg(", true), ("#[cfg(", false)] {
        let mut from = 0usize;
        while let Some(offset) = mask[from..].find(marker) {
            let start = from + offset;
            from = start + marker.len();
            let paren = start + marker.len() - 1;
            let Some(pred_end) = balanced_end(mask, paren, b'(', b')') else { continue };
            let src = text[paren + 1..pred_end - 1].trim();
            let pred = parse_pred(src).ok_or_else(|| src.to_string());
            let Some(attr_end) = balanced_end(mask, start + marker.find('[').unwrap_or(1), b'[', b']') else {
                continue;
            };
            let end = if inner {
                braces.iter().filter(|(o, c)| **o < start && start < **c).map(|(_, c)| *c).min().unwrap_or(mask.len())
            } else {
                let mut depth = 0i32;
                let mut end = mask.len();
                for (k, b) in bytes.iter().enumerate().skip(attr_end) {
                    match b {
                        b'(' | b'[' => depth += 1,
                        b')' | b']' => depth -= 1,
                        b'{' if depth == 0 => {
                            end = braces.get(&k).copied().unwrap_or(mask.len());
                            break;
                        }
                        b';' | b',' | b'}' if depth <= 0 => {
                            end = k;
                            break;
                        }
                        _ => {}
                    }
                }
                end
            };
            let start = if inner { braces.keys().filter(|o| **o < start && end == braces[o]).max().copied().unwrap_or(0) } else { start };
            gates.push(Gate { start, end, pred });
        }
    }
    gates
}

/// The body of the fn named `name` whose signature follows byte `from`, as a range of `mask`.
fn fn_body(mask: &str, braces: &BTreeMap<usize, usize>, from: usize, name: &str) -> Option<(usize, usize)> {
    let mut search = from;
    loop {
        let offset = mask[search..].find(name)?;
        let at = search + offset;
        search = at + name.len();
        let before = mask[..at].trim_end();
        let bounded = mask[search..].chars().next().is_some_and(|c| !is_ident_char(c));
        if bounded && before.ends_with("fn") && !before[..before.len() - 2].ends_with(is_ident_char) {
            let open = search + mask[search..].find('{')?;
            return Some((open, *braces.get(&open)?));
        }
    }
}

/// Whether `body` calls the fn `name` (a whole identifier followed by `(`).
fn calls(body: &str, name: &str) -> bool {
    let mut from = 0usize;
    while let Some(offset) = body[from..].find(name) {
        let at = from + offset;
        from = at + name.len();
        let bounded = !body[..at].ends_with(is_ident_char) && body[from..].trim_start().starts_with('(');
        if bounded {
            return true;
        }
    }
    false
}

/// The device entry points a body calls DIRECTLY: [`DEVICE_ENTRY_CALLS`] and `*_or_skip` helpers.
fn direct_device_entries(body: &str) -> Vec<String> {
    let mut found: Vec<String> =
        DEVICE_ENTRY_CALLS.iter().filter(|call| body.contains(*call)).map(|call| call.to_string()).collect();
    let mut from = 0usize;
    while let Some(offset) = body[from..].find("_or_skip") {
        let end = from + offset + "_or_skip".len();
        let start = body[..from + offset].rfind(|c: char| !is_ident_char(c)).map_or(0, |i| i + 1);
        let called = body[end..].trim_start().starts_with(['(', '!']);
        let whole = body[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if called && whole && start < from + offset {
            found.push(format!("{}(…)", &body[start..end]));
        }
        from = end;
    }
    found
}

/// Same-file fns that reach a device, each with why: those returning `Option<VulkanContext>` (the
/// boot helpers, under whatever name a file gave them), those whose body calls a device entry, and
/// — to a fixed point — those whose body calls one of these. The closure is what lets the rule see
/// a test that reaches `with_windowed_present` through a `run_showcase_dump`, or
/// `EnginePlugins::window` through a file's own `boot()`.
fn device_helpers(mask: &str, braces: &BTreeMap<usize, usize>) -> BTreeMap<String, String> {
    /// One `fn` item: its name, its signature with whitespace removed, and its body's braces.
    struct FnItem {
        name: String,
        signature: String,
        body: Option<(usize, usize)>,
    }
    let mut fns = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = mask[from..].find("fn ") {
        let at = from + offset;
        from = at + 3;
        if at > 0 && mask[..at].ends_with(is_ident_char) {
            continue;
        }
        let name: String = mask[at + 3..].trim_start().chars().take_while(|c| is_ident_char(*c)).collect();
        let Some(end) = mask[at..].find(['{', ';']) else { continue };
        let signature: String = mask[at..at + end].chars().filter(|c| !c.is_whitespace()).collect();
        let body = braces.get(&(at + end)).map(|close| (at + end, *close));
        if !name.is_empty() {
            fns.push(FnItem { name, signature, body });
        }
    }
    let mut helpers = BTreeMap::new();
    for FnItem { name, signature, body } in &fns {
        if signature.ends_with("->Option<VulkanContext>") {
            helpers.insert(name.clone(), "-> Option<VulkanContext>".to_string());
        } else if let Some((open, close)) = body
            && let Some(entry) = direct_device_entries(&mask[*open..=*close]).first()
        {
            helpers.insert(name.clone(), format!("reaches {entry}"));
        }
    }
    loop {
        let mut grew = false;
        for FnItem { name, body, .. } in &fns {
            let Some((open, close)) = body else { continue };
            if helpers.contains_key(name) {
                continue;
            }
            let reached = helpers.keys().find(|helper| calls(&mask[*open..=*close], helper)).cloned();
            if let Some(helper) = reached {
                helpers.insert(name.clone(), format!("reaches {helper}(…)"));
                grew = true;
            }
        }
        if !grew {
            return helpers;
        }
    }
}

/// The device entry points a test body calls, directly or through a same-file helper.
fn device_entries(body: &str, helpers: &BTreeMap<String, String>) -> Vec<String> {
    let mut found = direct_device_entries(body);
    for (helper, why) in helpers {
        if calls(body, helper) {
            found.push(format!("{helper}(…) [{why}]"));
        }
    }
    found.sort();
    found.dedup();
    found
}

/// What the source says about one site beyond its attribute.
struct Context {
    scope: Scope,
    /// A `cfg` over the test that is false natively and true under Miri.
    miri_gated: bool,
    /// `miri` appears in the site's `cfg_attr` predicate or in a `cfg` over it.
    mentions_miri: bool,
    /// The features that condition the test: named by a `cfg` over it that a feature switches
    /// on, or by its own `cfg_attr` predicate. Empty when no feature does.
    features: BTreeSet<String>,
    /// Device entry points the test body calls.
    device_entries: Vec<String>,
    /// Predicates the parser could not read, which the census refuses to guess about.
    unreadable: Vec<String>,
}

/// Reads [`Context`] for every site in one file.
fn contexts_in_text(text: &str, sites: &[Site]) -> Vec<Context> {
    let mask = code_mask(text);
    let braces = brace_pairs(&mask);
    let gates = gates_in(text, &mask, &braces);
    let helpers = device_helpers(&mask, &braces);
    sites
        .iter()
        .map(|site| {
            let mut ctx = Context {
                scope: Scope::Native,
                miri_gated: false,
                mentions_miri: false,
                features: BTreeSet::new(),
                device_entries: Vec::new(),
                unreadable: Vec::new(),
            };
            for gate in gates.iter().filter(|g| g.start <= site.offset && site.offset < g.end) {
                match &gate.pred {
                    Ok(pred) => {
                        ctx.miri_gated |= pred.is_miri_only();
                        ctx.mentions_miri |= pred.mentions("miri");
                        if pred.requires_a_feature() {
                            pred.features(&mut ctx.features);
                        }
                    }
                    Err(src) => ctx.unreadable.push(format!("cfg({src})")),
                }
            }
            if let Some(src) = &site.cfg {
                match parse_pred(src) {
                    Some(pred) => {
                        ctx.scope = Scope::of(&pred);
                        ctx.mentions_miri |= pred.mentions("miri");
                        pred.features(&mut ctx.features);
                    }
                    None => ctx.unreadable.push(format!("cfg_attr({src}, …)")),
                }
            } else if site.spelling == IgnoreSpelling::CfgAttr {
                ctx.unreadable.push("a cfg_attr predicate the census could not delimit".to_string());
            }
            if let Some(name) = &site.test_fn
                && let Some((open, close)) = fn_body(&mask, &braces, site.offset, name)
            {
                ctx.device_entries = device_entries(&mask[open..=close], &helpers);
            }
            ctx
        })
        .collect()
}

/// The features a crate declares in its `Cargo.toml` `[features]` table.
fn declared_features(manifest: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_table = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[features]";
            continue;
        }
        if !in_table || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
                out.insert(key.to_string());
            }
        }
    }
    out
}

/// Every rule a site breaks, each as one line naming the site.
///
/// Pure over its inputs, so the positive controls run the SAME rules on fixtures that the tree
/// runs on — never a test-local copy of them.
fn class_violations(site: &Site, ctx: &Context, declared: &BTreeSet<String>) -> Vec<String> {
    let name = site.test_fn.as_deref().unwrap_or("<unnamed>");
    let at = format!("{}:{} `{name}` [{}]", site.file, site.line, ctx.scope.name());
    let mut out = Vec::new();
    for unreadable in &ctx.unreadable {
        out.push(format!("{at}: the census cannot evaluate {unreadable}; extend `parse_pred` rather than guess"));
    }
    let Some(reason) = site.reason.as_deref() else {
        out.push(format!("{at}: the reason literal could not be decoded, so its class cannot be read"));
        return out;
    };
    for feature in features_named(reason) {
        if !feature.contains('/') && !declared.contains(&feature) {
            out.push(format!(
                "{at}: names `--features {feature}`, which the crate's [features] table does not declare"
            ));
        }
    }
    let classes = match parse_prefix(reason) {
        Ok((classes, _)) => classes,
        // The one scope that names no leg: the test runs in every debug run.
        Err(PrefixError::Missing) if ctx.scope == Scope::ReleaseOnly => return out,
        Err(e) => {
            let head: String = reason.chars().take(48).collect();
            let why = match e {
                PrefixError::Missing => "no `<class>: ` prefix".to_string(),
                PrefixError::Unknown(part) => format!("`{part}` is not a class"),
                PrefixError::Combination(head) => format!("`{head}` is not one of the class spellings"),
                PrefixError::NoProse => "a class with no prose after it".to_string(),
            };
            out.push(format!("{at}: {why} — reason starts {head:?}"));
            return out;
        }
    };
    let spelled = classes.join("+");
    let has = |set: &[&str]| classes.iter().any(|c| set.contains(c));

    if ctx.scope == Scope::MiriOnly && !(classes == ["miri-slow"] || classes == ["miri-unsupported"]) {
        out.push(format!(
            "{at}: `{spelled}` on a site ignored only under Miri — the test runs natively, so the class \
             says why Miri skips it: `miri-slow` or `miri-unsupported`"
        ));
    }
    if ctx.miri_gated && !has(&MIRI_CLASSES) {
        out.push(format!(
            "{at}: `{spelled}` on a test that exists only under Miri — natively it is not even \
             compiled, so no native leg can run it; the class is `miri-slow` or `miri-unsupported`"
        ));
    }
    if has(&MIRI_CLASSES) && !ctx.mentions_miri {
        out.push(format!(
            "{at}: `{spelled}` names Miri, but no `cfg` over this test and no `cfg_attr` on it mentions \
             `miri` — the Miri leg never sees this ignore"
        ));
    }
    let has_feature = classes.contains(&"feature");
    if !ctx.features.is_empty() && !has_feature {
        out.push(format!(
            "{at}: `{spelled}` on a test that {:?} conditions — it does not exist or does not run in \
             the default build, so the class starts `feature+`",
            ctx.features
        ));
    }
    if has_feature && ctx.features.is_empty() {
        out.push(format!(
            "{at}: `{spelled}` on a test no cargo feature conditions (no `cfg` over it and no \
             `cfg_attr` on it names one)"
        ));
    }
    if has_feature {
        let named = features_named(reason);
        for feature in ctx.features.difference(&named) {
            out.push(format!("{at}: `{spelled}` must name `--features {feature}` in its prose"));
        }
    }
    if !ctx.device_entries.is_empty() && !has(&DEVICE_CLASSES) && !has(&STANDALONE_CLASSES) {
        out.push(format!(
            "{at}: `{spelled}` on a test that calls {:?} — without a GPU that call returns early and \
             the test PASSES having measured nothing, so its class is a `gpu*` one",
            ctx.device_entries
        ));
    }
    out
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

/// The scanner's own positive control: the line-start states it must tell apart, each fixture run
/// through the shipped [`sites_in_text`] rather than through a test-local copy of it.
///
/// Every row is a shape this tree actually contains, and every row DISCRIMINATES: remove the piece
/// of the scanner it names and the row reds, either by losing a real site or by reporting a
/// phantom one.
#[test]
fn a_line_that_begins_inside_a_string_literal_is_not_a_site() {
    // (what the row pins, the fixture's lines, the 1-indexed lines that must be reported).
    let cases: &[(&str, &[&str], &[usize])] = &[
        (
            "a `\\`-continued reason string whose continuation line starts with `#[cfg_attr(`",
            &[
                "#[cfg(miri)]",
                "fn refuse_under_miri() {",
                "    panic!(",
                "        \"every counter this test reads is frozen at zero; a test that needs \\",
                "         #[cfg_attr(miri, ignore = ...)]\"",
                "    );",
                "}",
            ][..],
            &[][..],
        ),
        (
            "the line AFTER a multi-line string literal ends is ordinary code again",
            &[
                "static REASON: &str = \"first line of the literal",
                "#[ignore] — quoted, and inside the literal",
                "last line of the literal\";",
                "#[cfg_attr(miri, ignore = \"real: the literal closed on the line above\")]",
                "fn t() {}",
            ][..],
            &[4][..],
        ),
        (
            "a NESTED block comment holding a lone quote leaves the scanner in code afterwards",
            &[
                "/* outer /* nested */ still the outer comment, with a lone \" quote",
                "   and the outer comment ends here */",
                "#[cfg_attr(miri, ignore = \"real: after the comment\")]",
                "fn t() {}",
            ][..],
            &[3][..],
        ),
        (
            "a raw string's body may hold an odd number of quotes without ending it",
            &[
                "const SNIPPET: &str = r##\"",
                "#[ignore = \"quoted inside a raw string\"]",
                "a lone \" quote ends nothing here",
                "\"##;",
                "#[cfg_attr(miri, ignore = \"real: after the raw string\")]",
                "fn t() {}",
            ][..],
            &[5][..],
        ),
        (
            "a `'\"'` char literal opens no string, and `&'a T` is no unterminated one",
            &[
                "fn quote<'a>(s: &'a str) -> char {",
                "    let _ = s;",
                "    '\"'",
                "}",
                "#[ignore = \"solo: after a quote char literal\"]",
                "fn t() {}",
            ][..],
            &[5][..],
        ),
        (
            "`br#\"…\"#` and `c\"…\\\"…\"` end where their own rules say, not at the first quote",
            &[
                "const RAW_BYTES: &[u8] = br#\"a byte raw string with a \" inside\"#;",
                "const C_STR: &std::ffi::CStr = c\"a c-string with a \\\" escape\";",
                "#[cfg_attr(miri, ignore = \"real: after both literals\")]",
                "fn t() {}",
            ][..],
            &[3][..],
        ),
    ];

    for (what, lines, want) in cases {
        let src = lines.join("\n");
        let got: Vec<usize> = sites_in_text("fixture.rs", &src).iter().map(|s| s.line).collect();
        assert_eq!(
            got.as_slice(),
            *want,
            "the scanner read the wrong line starts for the case {what:?}; fixture:\n{src}"
        );
    }
}

/// The regression the scanner exists for, pinned at the two live lines that produced it.
///
/// A count is the only thing this gate publishes, so the two lines that made it wrong are named
/// here rather than left to the fixtures: fixtures prove the mechanism, and this proves the tree.
#[test]
fn the_two_continuation_lines_inside_panic_strings_are_not_counted() {
    // (file, 1-indexed continuation line whose first characters are `#[cfg_attr(`).
    const PHANTOMS: &[(&str, usize)] = &[
        ("crates/boyko_threadpool/tests/a6_panicked_scope_chunk_receipts.rs", 119),
        ("crates/boyko_threadpool/tests/block_allocation_receipts.rs", 459),
    ];

    let root = repo_root();
    for (rel, line) in PHANTOMS {
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("invariant: the census walks {rel}, which must exist: {e}"));
        let lines: Vec<&str> = text.lines().collect();
        let raw = lines
            .get(line - 1)
            .unwrap_or_else(|| panic!("{rel} is shorter than {line} lines; re-point this row"));

        // Anti-vacuity, and it is the whole weight of this test: without it the row passes the
        // day the phantom moves or the file is reformatted, reporting the scanner as working when
        // nothing was scanned. The line must STILL read as an attribute to the classifier.
        assert_ne!(
            classify(raw).0,
            IgnoreForm::NotAnIgnore,
            "{rel}:{line} no longer looks like an attribute to `classify`, so this row proves \
             nothing about the scanner — re-point it at the continuation line (search the file for \
             `must carry`) or delete it"
        );

        let sites = sites_in_text(rel, &text);
        assert!(
            !sites.iter().any(|s| s.line == *line),
            "{rel}:{line} is a continuation line of a `\\`-continued string inside `panic!`, and \
             the census counted it as a real ignore site — the published total is high by one per \
             line like it"
        );
        assert!(
            !sites.is_empty(),
            "{rel} carries real ignore sites and the census found none: the scanner skipped the \
             whole file, which would hide sites rather than phantoms"
        );
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

/// **The class clause.** Every reasoned site carries a class its own `cfg` allows (module doc,
/// "The class prefix").
///
/// Bare and empty reasons are the first clause's findings and are skipped here, so one defect is
/// reported once.
#[test]
fn every_site_carries_a_class_its_predicate_allows() {
    let (sites, _) = census();
    let root = repo_root();

    let mut manifests: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut violations = Vec::new();
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_scope: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut device_entry_sites, mut miri_only_tests, mut feature_conditioned) = (0usize, 0usize, 0usize);

    // `census` yields each file's sites together, in line order.
    for group in sites.chunk_by(|a, b| a.file == b.file) {
        let file = &group[0].file;
        let text = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("invariant: the census has just read {file}: {e}"));
        let krate = crate_of(file);
        let declared = manifests.entry(krate.clone()).or_insert_with(|| {
            let dir = if krate == "<root>" { root.clone() } else { root.join(&krate) };
            let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
                .unwrap_or_else(|e| panic!("invariant: {krate} has a Cargo.toml: {e}"));
            declared_features(&manifest)
        });
        let contexts = contexts_in_text(&text, group);
        for (site, ctx) in group.iter().zip(&contexts) {
            if site.form != IgnoreForm::Reasoned {
                continue;
            }
            violations.extend(class_violations(site, ctx, declared));
            *by_scope.entry(ctx.scope.name()).or_default() += 1;
            let spelled = match site.reason.as_deref().map(parse_prefix) {
                Some(Ok((classes, _))) => classes.join("+"),
                _ => "<none>".to_string(),
            };
            *by_class.entry(spelled).or_default() += 1;
            device_entry_sites += usize::from(!ctx.device_entries.is_empty());
            miri_only_tests += usize::from(ctx.miri_gated);
            feature_conditioned += usize::from(!ctx.features.is_empty());
        }
    }

    let classes: Vec<String> = by_class.iter().map(|(class, n)| format!("{class}={n}")).collect();
    let scopes: Vec<String> = by_scope.iter().map(|(scope, n)| format!("{scope}={n}")).collect();
    println!(
        "[ignore classes] {}; scopes: {}; rule inputs: {device_entry_sites} sites call a device \
         entry, {miri_only_tests} exist only under Miri, {feature_conditioned} are \
         feature-conditioned",
        classes.join(", "),
        scopes.join(", ")
    );

    // ── Non-vacuity of each rule. A rule whose input set is empty passes by construction, which is
    // the shape a broken detector takes: the body reader, the cfg reader and the gate ranges each
    // feed one rule, and each of those inputs is known to be non-empty on this tree.
    assert!(
        device_entry_sites > 0 && miri_only_tests > 0 && feature_conditioned > 0,
        "a consistency rule examined nothing ({device_entry_sites} device-entry sites, \
         {miri_only_tests} Miri-only tests, {feature_conditioned} feature-conditioned) — the reader \
         feeding it is broken, not the tree"
    );
    assert!(
        by_scope.contains_key(Scope::MiriOnly.name()) && by_scope.contains_key(Scope::Native.name()),
        "the scope evaluator put no site in {by_scope:?} — every cfg_attr(miri, …) site should be \
         miri-only and every plain site native"
    );

    let vocabulary: Vec<String> =
        IGNORE_CLASSES.iter().map(|(class, leg)| format!("  {class:<17} {leg}")).collect();
    assert!(
        violations.is_empty(),
        "{} ignore sites break the class rules:\n  {}\n\n\
         WHAT TO DO — read what the test NEEDS (its body, its boot call, its `cfg`, its module \
         header) and put `<class>: ` in front of the reason, keeping the prose. Do NOT choose by \
         keyword: CLAUDE.md records a keyword classifier that misfiled 8 of 10 sites, every one \
         toward a green that measured nothing. The classes and their legs:\n{}\n\
         Spellings: `<class>`, `feature+<class>`, `gpu-windowed+gpu-cap`, \
         `feature+gpu-windowed+gpu-cap`; `generator`, `deferred` and `flaky` stand alone.",
        violations.len(),
        violations.join("\n  "),
        vocabulary.join("\n")
    );
}

/// The prefix parser's own positive control: every shape it accepts and every shape it must not.
#[test]
fn the_class_grammar_classifies_every_shape() {
    use PrefixError::{Combination, Missing, NoProse, Unknown};

    for (class, _) in IGNORE_CLASSES {
        let reason = format!("{class}: prose");
        assert_eq!(parse_prefix(&reason), Ok((vec![class], "prose")), "a lone `{class}` must parse");
    }

    let accepted: &[(&str, &[&str])] = &[
        ("feature: --features hwrt; a device-free test that exists only with the feature", &["feature"]),
        ("feature+gpu-cap: --features hwrt; a real RT device", &["feature", "gpu-cap"]),
        ("feature+gpu-windowed+gpu-cap: --features hwrt; windowed, on RT", &["feature", "gpu-windowed", "gpu-cap"]),
        ("gpu-windowed+gpu-cap: ray query and a window", &["gpu-windowed", "gpu-cap"]),
        ("feature+miri-slow: --features tb-neg-m2w, under Miri only", &["feature", "miri-slow"]),
        // A second colon belongs to the prose.
        ("miri-slow: Miri wall-time: 64 cases", &["miri-slow"]),
    ];
    for (reason, want) in accepted {
        assert_eq!(parse_prefix(reason).map(|(c, _)| c), Ok(want.to_vec()), "{reason:?} must parse as {want:?}");
    }

    let rejected: &[(&str, PrefixError)] = &[
        // No prefix at all — the shape of 177 sites before the migration.
        ("needs a real windowed GPU device; run with --test-threads=1", Missing),
        ("Miri wall-time: 64 cases", Missing),
        // Prefixes outside the vocabulary — the five the migration re-prefixed, and a case slip.
        ("calibration: builds 3 legs x 3 link configurations", Unknown("calibration".into())),
        ("tractability: 100k rows through the apply window", Unknown("tractability".into())),
        ("instrument: reads the recording allocator's tape", Unknown("instrument".into())),
        ("M2: trilinear stepping deferred to the JCGT cubic", Unknown("M2".into())),
        ("miri-arm: decides a Tree-Borrows property", Unknown("miri-arm".into())),
        ("GPU: needs a device", Unknown("GPU".into())),
        ("feature+: dangling", Unknown(String::new())),
        // Not the `<class>: ` shape.
        ("gpu : needs a device", Missing),
        ("gpu:needs a device", Missing),
        ("gpu:", NoProse),
        ("gpu:   ", NoProse),
        // Classes that exist, combined in a way that is not a spelling.
        ("deferred+gpu: red by design", Combination("deferred+gpu".into())),
        ("feature+generator: emits a table", Combination("feature+generator".into())),
        ("gpu+feature: order", Combination("gpu+feature".into())),
        ("gpu+gpu-windowed: redundant", Combination("gpu+gpu-windowed".into())),
        ("gpu-cap+gpu-windowed: the one spelling is the other order", Combination("gpu-cap+gpu-windowed".into())),
        ("feature+feature: twice", Combination("feature+feature".into())),
        ("solo+slow: two device-free legs", Combination("solo+slow".into())),
    ];
    for (reason, want) in rejected {
        assert_eq!(parse_prefix(reason).map(|(c, _)| c), Err(want.clone()), "{reason:?} must be refused");
    }

    let named: &[(&str, &[&str])] = &[
        ("feature+gpu-cap: requires a real RT GPU (run: --features hwrt -- --ignored)", &["hwrt"]),
        ("under an arm that only `--features tb-neg-m2w` builds", &["tb-neg-m2w"]),
        ("--features=a,b and later --features c", &["a", "b", "c"]),
        ("no flag here", &[]),
    ];
    for (reason, want) in named {
        let got: Vec<String> = features_named(reason).into_iter().collect();
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want, "features_named({reason:?})");
    }
}

/// The predicate evaluator's own positive control, on every predicate shape this tree spells.
#[test]
fn a_cfg_predicate_sets_the_scope() {
    let cases: &[(&str, Scope)] = &[
        ("miri", Scope::MiriOnly),
        ("all(miri, unix)", Scope::MiriOnly),
        ("any(miri, debug_assertions)", Scope::Native),
        ("not(debug_assertions)", Scope::ReleaseOnly),
        ("not(all(miri, feature = \"tb-neg-m2w\"))", Scope::Native),
        ("feature = \"ignore_slow\"", Scope::FeatureOnly),
        ("all(windows, not(miri))", Scope::Native),
    ];
    for (src, want) in cases {
        let pred = parse_pred(src).unwrap_or_else(|| panic!("{src:?} must parse"));
        assert_eq!(Scope::of(&pred), *want, "scope of cfg_attr({src}, ignore …)");
    }

    for bad in ["miri,", "not(a, b)", "frobnicate(miri)", "feature = ", "", "all(miri"] {
        assert_eq!(parse_pred(bad), None, "{bad:?} is not a predicate and must not be guessed at");
    }

    let miri_only = parse_pred("miri").expect("parses");
    assert!(miri_only.is_miri_only() && !miri_only.requires_a_feature());
    let not_miri = parse_pred("not(miri)").expect("parses");
    assert!(!not_miri.is_miri_only());
    let hwrt = parse_pred("all(windows, feature = \"hwrt\")").expect("parses");
    assert!(hwrt.requires_a_feature() && !hwrt.is_miri_only());
    let tb_neg = parse_pred("all(miri, feature = \"tb-neg-m2w\")").expect("parses");
    assert!(tb_neg.requires_a_feature() && tb_neg.is_miri_only());
}

/// The reason reader's own positive control: a class can only be checked if the reason is read
/// exactly as rustc reads it, across the multi-line shapes this tree uses.
#[test]
fn the_reason_and_the_predicate_are_read_as_rustc_reads_them() {
    let cases: &[(&str, &str, Option<&str>)] = &[
        ("#[ignore = \"gpu: one line\"]\nfn t() {}\n", "gpu: one line", None),
        (
            "#[ignore = \"gpu: continued \\\n            over two lines\"]\nfn t() {}\n",
            "gpu: continued over two lines",
            None,
        ),
        (
            "#[cfg_attr(\n    miri,\n    ignore = \"miri-slow: split \\\n              attribute\"\n)]\nfn t() {}\n",
            "miri-slow: split attribute",
            Some("miri"),
        ),
        (
            "#[cfg_attr(not(all(miri, feature = \"x\")), ignore = \"a \\\"quoted\\\" word, (paren\")]\nfn t() {}\n",
            "a \"quoted\" word, (paren",
            Some("not(all(miri, feature = \"x\"))"),
        ),
        ("#[ignore = r#\"slow: raw \"quote\"\"#]\nfn t() {}\n", "slow: raw \"quote\"", None),
        ("#[test] #[ignore = \"solo: same line\"]\nfn t() {}\n", "solo: same line", None),
    ];
    for (fixture, want_reason, want_cfg) in cases {
        let sites = sites_in_text("fixture.rs", fixture);
        assert_eq!(sites.len(), 1, "fixture must hold one site:\n{fixture}");
        assert_eq!(sites[0].reason.as_deref(), Some(*want_reason), "reason of:\n{fixture}");
        assert_eq!(sites[0].cfg.as_deref(), *want_cfg, "predicate of:\n{fixture}");
    }
}

/// The consistency rules' own positive control: each rule on the shape it exists for, run through
/// the shipped reader and rules, with a clean twin beside every red so neither half is vacuous.
#[test]
fn the_consistency_rules_fire_on_the_shapes_they_exist_for() {
    const BOOT: &str = "fn boot_or_skip() -> Option<VulkanContext> { None }\n";
    let declared: BTreeSet<String> = ["hwrt".to_string(), "tb-neg-m2w".to_string()].into();
    // (what, fixture, substrings each of which must appear in some violation; empty = clean).
    let cases: Vec<(&str, String, &[&str])> = vec![
        (
            "a `solo` test that boots through `boot_or_skip` (the CLAUDE.md trap)",
            format!("{BOOT}#[test]\n#[ignore = \"solo: run alone\"]\nfn t() {{\n    let Some(_c) = boot_or_skip() else {{ return }};\n}}\n"),
            &["boot_or_skip(…)"],
        ),
        (
            "the same test as `gpu`",
            format!("{BOOT}#[test]\n#[ignore = \"gpu: a device\"]\nfn t() {{\n    let Some(_c) = boot_or_skip() else {{ return }};\n}}\n"),
            &[],
        ),
        (
            "a boot helper under another name, found by its return type",
            "fn open() -> Option<VulkanContext> { None }\n#[test]\n#[ignore = \"slow: x\"]\nfn t() { let _ = open(); }\n"
                .to_string(),
            &["open(…) [-> Option<VulkanContext>]"],
        ),
        (
            "a device reached through two same-file helpers",
            "fn boot() -> App { let mut app = App::new(); app.add_plugins(EnginePlugins::window(\"t\", 1, 1)); app }
             fn run_dump() { boot().run(); }
#[test]
#[ignore = \"solo: x\"]
fn t() { run_dump(); }
"
                .to_string(),
            &["run_dump(…) [reaches boot(…)]"],
        ),
        (
            "a windowed `boyko_app` test labelled device-free",
            "#[test]\n#[ignore = \"slow: x\"]\nfn t() { app.add_plugins(EnginePlugins::window(\"t\", 1, 1)); }\n".to_string(),
            &["EnginePlugins::window"],
        ),
        (
            "a device `generator` names no leg, so no leg can sweep it in",
            format!("{BOOT}#[test]\n#[ignore = \"generator: dumps a table\"]\nfn t() {{ let _ = boot_or_skip(); }}\n"),
            &[],
        ),
        (
            "the boot call in a SIBLING test does not reach this one",
            format!("{BOOT}#[test]\nfn u() {{ let _ = boot_or_skip(); }}\n#[test]\n#[ignore = \"solo: x\"]\nfn t() {{}}\n"),
            &[],
        ),
        (
            "a plain `slow` test in a `#![cfg(miri)]` file (the CLAUDE.md trap)",
            "#![cfg(miri)]\n#[test]\n#[ignore = \"slow: x\"]\nfn t() {}\n".to_string(),
            &["exists only under Miri"],
        ),
        (
            "the same test as `miri-slow`",
            "#![cfg(miri)]\n#[test]\n#[ignore = \"miri-slow: x\"]\nfn t() {}\n".to_string(),
            &[],
        ),
        (
            "a plain `miri-slow` test that nothing ties to Miri",
            "#[test]\n#[ignore = \"miri-slow: x\"]\nfn t() {}\n".to_string(),
            &["no `cfg` over this test"],
        ),
        (
            "`cfg_attr(miri, \"slow: …\")` — a non-Miri class at Miri-only scope",
            "#[test]\n#[cfg_attr(miri, ignore = \"slow: x\")]\nfn t() {}\n".to_string(),
            &["ignored only under Miri"],
        ),
        (
            "`cfg_attr(miri, \"miri-unsupported: …\")`",
            "#[test]\n#[cfg_attr(miri, ignore = \"miri-unsupported: a child process\")]\nfn t() {}\n".to_string(),
            &[],
        ),
        (
            "release-only scope: a reason with no class names no leg and passes",
            "#[test]\n#[cfg_attr(not(debug_assertions), ignore = \"the guard is a debug_assert\")]\nfn t() {}\n"
                .to_string(),
            &[],
        ),
        (
            "release-only scope: a prefix outside the vocabulary still reds",
            "#[test]\n#[cfg_attr(not(debug_assertions), ignore = \"calibration: x\")]\nfn t() {}\n".to_string(),
            &["is not a class"],
        ),
        (
            "a `gpu` test in a feature-gated file",
            "#![cfg(feature = \"hwrt\")]\n#[test]\n#[ignore = \"gpu: x\"]\nfn t() {}\n".to_string(),
            &["starts `feature+`"],
        ),
        (
            "`feature+gpu` naming the gating feature",
            "#![cfg(feature = \"hwrt\")]\n#[test]\n#[ignore = \"feature+gpu: --features hwrt; x\"]\nfn t() {}\n"
                .to_string(),
            &[],
        ),
        (
            "`feature+gpu` whose prose does not name the flag",
            "#![cfg(feature = \"hwrt\")]\n#[test]\n#[ignore = \"feature+gpu: x\"]\nfn t() {}\n".to_string(),
            &["must name `--features hwrt`"],
        ),
        (
            "a fn-level `#[cfg(feature)]`",
            "#[cfg(feature = \"hwrt\")]\n#[test]\n#[ignore = \"gpu-windowed: x\"]\nfn t() {}\n".to_string(),
            &["starts `feature+`"],
        ),
        (
            "a `cfg(feature)` over a module reaches the tests inside it",
            "#[cfg(feature = \"hwrt\")]\nmod m {\n    #[test]\n    #[ignore = \"gpu: x\"]\n    fn t() {}\n}\n".to_string(),
            &["starts `feature+`"],
        ),
        (
            "a `cfg(feature)` on a SIBLING fn does not reach this one",
            "#[cfg(feature = \"hwrt\")]\nfn helper() {}\n#[test]\n#[ignore = \"gpu: x\"]\nfn t() {}\n".to_string(),
            &[],
        ),
        (
            "`feature` on a test that no feature conditions",
            "#[test]\n#[ignore = \"feature: --features hwrt; x\"]\nfn t() {}\n".to_string(),
            &["no cargo feature conditions"],
        ),
        (
            "a feature the crate does not declare",
            "#![cfg(feature = \"nosuch\")]\n#[test]\n#[ignore = \"feature+gpu: --features nosuch; x\"]\nfn t() {}\n"
                .to_string(),
            &["does not declare"],
        ),
        (
            "the tb-neg shape: the feature and Miri in the site's own predicate",
            "#[test]\n#[cfg_attr(not(all(miri, feature = \"tb-neg-m2w\")), ignore = \"feature+miri-slow: only \
             `--features tb-neg-m2w` builds the arm\")]\nfn t() {}\n"
                .to_string(),
            &[],
        ),
        (
            "a predicate the evaluator cannot read reds rather than being guessed at",
            "#[test]\n#[cfg_attr(frobnicate(miri), ignore = \"miri-slow: x\")]\nfn t() {}\n".to_string(),
            &["cannot evaluate"],
        ),
    ];
    for (what, fixture, want) in &cases {
        let sites = sites_in_text("fixture.rs", fixture);
        assert_eq!(sites.len(), 1, "{what}: the fixture must hold exactly one site:\n{fixture}");
        let contexts = contexts_in_text(fixture, &sites);
        let got = class_violations(&sites[0], &contexts[0], &declared);
        if want.is_empty() {
            assert!(got.is_empty(), "{what}: expected no violation, got {got:#?}\n{fixture}");
        }
        for needle in *want {
            assert!(
                got.iter().any(|g| g.contains(needle)),
                "{what}: expected a violation containing {needle:?}, got {got:#?}\n{fixture}"
            );
        }
    }
}
