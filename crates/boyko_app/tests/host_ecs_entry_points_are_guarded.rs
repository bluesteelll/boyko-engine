//! Defect A6, validation row **P10** — the windowed host reaches the ECS only through the guarded
//! wrappers.
//!
//! # The claim
//!
//! `runner.rs` owns eight routes into the ECS (`app.finish()` once, `app.update_with_delta(dt)` per
//! frame, and six `app.world_mut().run_system(..)` one-shots). A panic on any of them used to unwind
//! through `run_windowed`'s own frame — or, worse, leave the window up with a blocked scheduler. The
//! fix routes every one of them through `guarded(..)` via three thin wrappers (`guarded_finish`,
//! `guarded_update`, `guarded_run_system`), so the three call shapes each occur EXACTLY ONCE in the
//! crate: inside their wrapper.
//!
//! # Why a token count and not a line check
//!
//! A formatter can split a wrapped call across lines; it cannot change how many times a token
//! occurs. Exactly-once is the anti-vacuity floor AND the ninth-site gate in one predicate: a renamed
//! method gives 0, a new direct call gives 2.
//!
//! # Why the whole `src/` and not `runner.rs`
//!
//! The census's job is the NINTH site, and a ninth site would arrive in a new host file as easily as
//! beside the eight. So every `*.rs` under this crate's `src/` is walked. It is crate-scoped on
//! purpose: the claim is about the WINDOWED host; `boyko_demo` and the headless harnesses keep the
//! panic-unwinds-to-`main` behaviour, so a `run_system` there is not a ninth site.
//!
//! # The forbidden doors
//!
//! `app.update()`, `.run_n(`, `.run_n_with_delta(` and `run_system_once(` reach the same executor
//! and are NOT wrapped. `update()` is the DEFAULT host door, so a ninth site is at least as likely to
//! spell itself that way. Each must occur zero times — and a zero-count assertion is a check that
//! cannot fail unless its needle can match, so each needle carries a positive control on a synthetic
//! input.
//!
//! **Stated limit**: `app.update()` is receiver-qualified (bare `update(` collides with
//! `hasher.update(chunk)`), so a call through a binding not named `app` evades it. The exactly-once
//! needles are the primary gate; the forbidden set is the second line.
//!
//! # What the census CANNOT claim
//!
//! It reads text. Each line is truncated at its first `//`, which removes comments (and, in the one
//! shape that could occur, a `//` inside a string literal). Truncation can only REMOVE matches, so it
//! can only produce a false red, never a false green. `/* */` block comments are not stripped; the
//! crate carries none on these tokens, and a hit inside one would be a false red naming its line.
//!
//! # What makes it RED
//!
//! - **M15** (`guarded_update(app, dt, "frame")` → `app.update_with_delta(dt)`): `update_with_delta(`
//!   occurs twice.
//! - **M19** (`guarded_update(app, dt, "frame")` → `app.update()`): the forbidden-door clause, with
//!   the exactly-once clause still satisfied by the wrapper body.
//! - A wrapper inlined away, or a call moved out of its wrapper: the locality clause.
//! - A walk that found fewer than 25 files, or a matcher that drifted: the floors and self-checks.
//!
//! This test is deliberately NOT `#[cfg(windows)]`, although the wrappers are: it is a source-text
//! census and must run on every platform.

use std::path::{Path, PathBuf};

/// The three ECS entry points the windowed host owns, each paired with the ONLY wrapper allowed to
/// contain it.
const GUARDED_SHAPES: [(&str, &str); 3] = [
    ("app.finish()", "guarded_finish"),
    ("update_with_delta(", "guarded_update"),
    ("run_system(", "guarded_run_system"),
];

/// Entry points that reach the same executor and are not wrapped. Each must occur zero times.
const FORBIDDEN_DOORS: [&str; 4] = ["app.update()", ".run_n(", ".run_n_with_delta(", "run_system_once("];

/// The guard core. At least three occurrences: one per wrapper body. `fn guarded<R>(` is not an
/// occurrence (`guarded<` is not `guarded(`).
const GUARD_CORE: &str = "guarded(";

/// Vacuity floor for the walk: `crates/boyko_app/src` holds 33 files at the design's HEAD.
const MIN_SCANNED_FILES: usize = 25;

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Byte offsets at which `needle` occurs as a call-shaped token.
///
/// LEFT boundary required, and it is the whole discrimination: `guarded_run_system(` must not count
/// as `run_system(`. It applies only when the needle itself BEGINS with an identifier byte — a needle
/// that begins with `.` (`.run_n(`) carries its own left boundary, and requiring another would demand
/// that the RECEIVER end in a non-identifier byte, which no method call does.
///
/// RIGHT boundary NOT checked, and that is the difference from `boyko_log`'s `code_registry.rs`,
/// whose needles are bare identifiers. Every needle here ends in `(` or `)`, which IS the right
/// boundary; requiring another one would demand that the first argument be a delimiter. The shape is
/// asserted at call time so the rule cannot drift from the needles.
fn token_positions(hay: &str, needle: &str) -> Vec<usize> {
    let nb = needle.as_bytes();
    assert!(
        matches!(nb.last(), Some(b'(' | b')')),
        "matcher invariant: every needle ends in `(` or `)` -- {needle:?} does not, so the \
         no-right-boundary rule would accept a prefix of a longer identifier"
    );
    let needs_left = is_ident_byte(nb[0]);
    let hb = hay.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let at = from + rel;
        if !needs_left || at == 0 || !is_ident_byte(hb[at - 1]) {
            out.push(at);
        }
        from = at + 1;
    }
    out
}

/// Every `*.rs` under `dir`, recursively, sorted for a stable report.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("test setup: cannot read directory {}: {e}", d.display()));
        for entry in entries {
            let path = entry.expect("test setup: directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// One line with everything from its first `//` removed.
fn code_part(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// `true` iff `line` declares `fn <name>` (followed by `(` or `<`).
fn declares_fn(line: &str, name: &str) -> bool {
    let decl = format!("fn {name}");
    line.match_indices(&decl).any(|(at, _)| {
        let after = line.as_bytes().get(at + decl.len()).copied();
        let before_ok = at == 0 || !is_ident_byte(line.as_bytes()[at - 1]);
        before_ok && matches!(after, Some(b'(' | b'<'))
    })
}

/// A hit: file, 1-based line number, the nearest `fn ` line at or before it (same file).
struct Hit {
    file: String,
    line_no: usize,
    enclosing_fn: Option<String>,
}

/// Every occurrence of `needle` across `files`, with its enclosing-`fn` line.
fn census(files: &[(String, Vec<String>)], needle: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (name, lines) in files {
        let mut last_fn: Option<String> = None;
        for (idx, line) in lines.iter().enumerate() {
            if line.contains("fn ") {
                last_fn = Some(line.trim().to_owned());
            }
            for _ in token_positions(line, needle) {
                hits.push(Hit { file: name.clone(), line_no: idx + 1, enclosing_fn: last_fn.clone() });
            }
        }
    }
    hits
}

fn render(hits: &[Hit]) -> String {
    hits.iter()
        .map(|h| {
            format!(
                "  {}:{} (nearest fn line: {})",
                h.file,
                h.line_no,
                h.enclosing_fn.as_deref().unwrap_or("<none>")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// P10 — see the module doc.
#[test]
fn the_windowed_host_reaches_the_ecs_only_through_the_guarded_wrappers() {
    // ── The matcher's self-checks, one per failure polarity. ──────────────────────────────────
    assert_eq!(
        token_positions("guarded_run_system(&mut app, \"x\", f)", "run_system(").len(),
        0,
        "self-check (left boundary dropped -- the `str::find` polarity): `guarded_run_system(` must \
         not count as `run_system(`"
    );
    assert_eq!(
        token_positions("app.world_mut().run_system(f)", "run_system(").len(),
        1,
        "self-check (right boundary imposed -- the `code_registry` polarity): a call whose first \
         argument begins with an identifier byte must still count"
    );
    assert_eq!(
        token_positions("guarded(\"a\", || app.finish()); guarded(\"b\", || app.finish())", "app.finish()")
            .len(),
        2,
        "self-check (per-line counting): two matches on one line are two occurrences"
    );
    // Positive controls for the zero-count needles: a needle that can never match is a check that
    // can never fail.
    assert_eq!(token_positions("app.update();", "app.update()").len(), 1, "forbidden-door control: app.update()");
    assert_eq!(token_positions("app.run_n(3);", ".run_n(").len(), 1, "forbidden-door control: .run_n(");
    assert_eq!(
        token_positions("app.run_n_with_delta(3, dt);", ".run_n_with_delta(").len(),
        1,
        "forbidden-door control: .run_n_with_delta("
    );
    assert_eq!(
        token_positions("w.run_system_once(f);", "run_system_once(").len(),
        1,
        "forbidden-door control: run_system_once("
    );
    // The two `run_n` needles are disjoint, and `run_system(` does not see `run_system_once(`.
    assert_eq!(token_positions("app.run_n_with_delta(3, dt);", ".run_n(").len(), 0, "`.run_n(` must not match `.run_n_with_delta(`");
    assert_eq!(token_positions("w.run_system_once(f);", "run_system(").len(), 0, "`run_system(` must not match `run_system_once(`");
    // And the locality helper: a wrapper declaration is recognised, a prefix of it is not.
    assert!(declares_fn("fn guarded_update(app: &mut App, dt: Duration, stage: &'static str) {", "guarded_update"));
    assert!(!declares_fn("fn guarded_update_all(app: &mut App) {", "guarded_update"));

    // ── The walk. ─────────────────────────────────────────────────────────────────────────────
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let paths = rust_files(&src);
    let files: Vec<(String, Vec<String>)> = paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("test setup: cannot read {}: {e}", p.display()));
            let rel = p.strip_prefix(&src).unwrap_or(p).display().to_string().replace('\\', "/");
            (rel, text.lines().map(|l| code_part(l).to_owned()).collect())
        })
        .collect();
    println!("[P10] scanned {} files under {}", files.len(), src.display());
    assert!(
        files.len() >= MIN_SCANNED_FILES,
        "P10: the walk found {} `*.rs` files under {}; at least {MIN_SCANNED_FILES} were expected -- \
         a census over a truncated tree is green from emptiness",
        files.len(),
        src.display()
    );

    // ── Exactly once, inside the matching wrapper. ────────────────────────────────────────────
    for (needle, wrapper) in GUARDED_SHAPES {
        let hits = census(&files, needle);
        println!("[P10] {needle:<20} occurrences = {}", hits.len());
        assert_eq!(
            hits.len(),
            1,
            "P10: `{needle}` must occur EXACTLY ONCE across the host crate -- inside `{wrapper}`. \
             0 means the method was renamed and this census is matching nothing; 2 or more means a \
             route into the ECS that bypasses `guarded` (a ninth site, or an unwrapped one):\n{}",
            render(&hits)
        );
        let hit = &hits[0];
        let inside = hit.enclosing_fn.as_deref().is_some_and(|l| declares_fn(l, wrapper));
        assert!(
            inside,
            "P10: the one `{needle}` is not inside `{wrapper}` -- the call reaches the ECS without \
             the frame-boundary catch:\n{}",
            render(&hits)
        );
    }

    let core = census(&files, GUARD_CORE);
    println!("[P10] {GUARD_CORE:<20} occurrences = {}", core.len());
    assert!(
        core.len() >= 3,
        "P10: `{GUARD_CORE}` occurs {} times; each of the three wrappers must call it:\n{}",
        core.len(),
        render(&core)
    );

    // ── The forbidden doors. ──────────────────────────────────────────────────────────────────
    for door in FORBIDDEN_DOORS {
        let hits = census(&files, door);
        println!("[P10] forbidden {door:<20} occurrences = {}", hits.len());
        assert!(
            hits.is_empty(),
            "`{door}` reaches the ECS without a guard. Add a `guarded_*` wrapper for it (Decision 8) \
             and move it into the exactly-once set, or route the call through `guarded_update`.\n{}",
            render(&hits)
        );
    }
}
