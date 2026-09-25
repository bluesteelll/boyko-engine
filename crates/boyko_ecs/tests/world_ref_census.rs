//! PC-24 guard G-b: a source census of every place a raw world pointer is named or minted.
//!
//! PC-24 was a worker forming `&mut EcsMaster` (`UnsafeEcsCell::archetype_ptr_mut` via
//! `world_mut()`) while another worker's `Commands::spawn` RMW'd the entity reservoir that lives
//! inline in the world. Invariant W0 (`unsafe_ecs_cell.rs` module docs) forbids any worker-side
//! `&mut` over the world. This census keeps the SHAPES that could reintroduce such a reference
//! pinned, so a new one is reviewed with a reason instead of arriving silently:
//!
//! - **R2′(a)** — an exact census of the cell's private `ptr` field. `UnsafeEcsCell::ptr` is
//!   private, so only `unsafe_ecs_cell.rs` can name it (the compiler enforces that), and an exact
//!   census of the token in that one file sees every use of the cell's world pointer: a field
//!   projection followed by a `&mut self` method, an alias, a destructure, a split `(*self` /
//!   `.ptr)`. A grep cannot know a method's receiver type, so the census does not classify: any
//!   new row is RED, and so is a missing one (a deleted row is never a smaller green).
//! - **R2′(b)** — the non-test part of `unsafe_ecs_cell.rs` declares no module: a
//!   `#[path = "…"] mod x;` child could name the private field from another file.
//! - **R1′** — per-file counts of the spellings that mint or name a raw world pointer
//!   (`NonNull::from(`, `*mut/*const EcsMaster` / `NonNull<EcsMaster>`, `&raw const/mut *…`) over
//!   `src/**` equal an allowlist whose every row carries its reason class; `as *mut/*const Self`
//!   must stay absent.
//! - **R4** — the `.world_mut()` call lines in non-test `src/**` are exactly the dispatcher-solo
//!   sites, each with its gate.
//! - **R5** — no `.world_mut()` call in the non-test part of `unsafe_ecs_cell.rs`: the exact
//!   PC-24 shape.
//!
//! It is a grep census and knows it. R1′ does not see a mint through an inferred cast
//! (`world as *const _ as *mut _`), `ptr::from_mut`, a helper or a generic; a macro defined
//! elsewhere and invoked in `unsafe_ecs_cell.rs` could expand to a field access no line spells.
//! The behavioural backstops are the `world_mut` tripwire (G-a) and the `pc24_` Miri matrix in
//! `unsafe_ecs_cell.rs`, run on both borrow models.
//!
//! Scope: non-test code only. A file is cut at the first `#[cfg(test)]` that applies to a
//! `mod … {`; a file declared `#[cfg(test)] mod x;` is skipped whole; comments are removed
//! before any rule looks at a line. Every rule is a function over `&str` input, and each has a
//! fixture that must RED at a named line — so an empty glob, a broken reader or a rule that
//! cannot fire fails here instead of passing.
//!
//! Reads `src/` from disk, which Miri isolation rejects by aborting the whole test binary, hence
//! `cfg(not(miri))`.
#![cfg(not(miri))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The cell file, relative to `crates/boyko_ecs/src`.
const CELL: &str = "ecs/core/system/unsafe_ecs_cell.rs";

/// R2′(a): every use of the cell's private `ptr` field in the non-test part of the cell file, as
/// (enclosing item, whitespace-collapsed line). Rows 9 and 10 are the only field-then-method
/// shapes admitted, because their receivers are `Option` projections (`as_deref`,
/// `as_deref_mut`), and row 10 runs dispatcher-solo (`CpuExclusive`).
const R2A_ALLOW: [(&str, &str); 10] = [
    // The field itself.
    ("struct UnsafeEcsCell", "ptr: *mut EcsMaster,"),
    // Constructors.
    ("new_mutable", "ptr: world as *mut EcsMaster,"),
    ("new_readonly", "ptr: world as *const EcsMaster as *mut EcsMaster,"),
    // Shared whole-world reference (W3): frozen bytes only, W0.
    ("world", "unsafe { &*self.ptr }"),
    // Whole-world `&mut`: dispatcher-solo only, guarded by the W0 tripwire and by R4.
    ("world_mut", "unsafe { &mut *self.ptr }"),
    // Shared field projection.
    ("resources", "unsafe { &(*self.ptr).resources }"),
    // Raw projections: no reference to the world or to `EntityMaster` at all.
    (
        "entity_counter",
        "let reservoir = unsafe { &raw const (*self.ptr).entity_master.reservoir };",
    ),
    (
        "entity_inland_store",
        "let store_ptr = unsafe { &raw const (*self.ptr).entity_master.entities_inland };",
    ),
    // Field then shared `Option` projection.
    ("nonsend_resources", "let slab = unsafe { (*self.ptr).nonsend_resources.as_deref() }?;"),
    // Field then `Option` projection, dispatcher-solo (`CpuExclusive`, W7).
    (
        "nonsend_resources_mut",
        "let slab = unsafe { (*self.ptr).nonsend_resources.as_deref_mut() }?;",
    ),
];

/// R1′: per-file counts `[S1, S2, S5]` over non-test `src/**`, with the reason each file may
/// name or mint a raw world pointer.
///
/// - S1 `NonNull::from(`
/// - S2 `*mut EcsMaster` | `*const EcsMaster` | `NonNull<EcsMaster>`
/// - S5 `&raw const *` | `&raw mut *` | `&raw const (*` | `&raw mut (*`
///
/// `app/app.rs`'s `&raw const *field` (`:1275`) is not a row: it sits inside
/// `#[cfg(test)] #[cfg(not(miri))] mod tests {`, which the test-module cut sees through the
/// second attribute.
const R1_ALLOW: [(&str, [usize; 3], &str); 18] = [
    (CELL, [0, 4, 2], "cell: the private world pointer, W0"),
    (
        "ecs/core/system/dispatcher_token.rs",
        [0, 4, 0],
        "dispatcher: minted only with no worker live (S1')",
    ),
    ("ecs/core/commands/command.rs", [0, 2, 0], "apply window (SCH7)"),
    ("ecs/core/commands/command_queue.rs", [1, 2, 0], "apply window (SCH7)"),
    ("ecs/core/commands/insert_command.rs", [6, 0, 0], "apply window (SCH7)"),
    ("ecs/core/commands/migration_helpers.rs", [9, 0, 0], "apply window (SCH7)"),
    ("ecs/core/commands/spawn_at_command.rs", [2, 0, 0], "apply window (SCH7)"),
    ("ecs/core/clone/materialize.rs", [2, 0, 0], "apply window (SCH7)"),
    ("ecs/core/component/hooks/deferred_master.rs", [0, 4, 0], "apply window (SCH7)"),
    ("ecs/core/component/hooks/dispatch.rs", [0, 1, 0], "apply window (SCH7)"),
    ("ecs/core/component/observers/dispatch.rs", [0, 1, 0], "apply window (SCH7)"),
    ("ecs/core/component/observers/entity_store.rs", [0, 2, 0], "apply window (SCH7)"),
    ("ecs/core/component/observers/trigger.rs", [0, 1, 0], "apply window (SCH7)"),
    (
        "ecs/core/ecs_master/component_api.rs",
        [2, 0, 0],
        "`&mut self` in `impl EcsMaster`: no worker holds one (R2', R4, G-a)",
    ),
    (
        "ecs/core/ecs_master/entity_api.rs",
        [4, 0, 0],
        "`&mut self` in `impl EcsMaster`: no worker holds one (R2', R4, G-a)",
    ),
    (
        "ecs/core/ecs_master/observer_api.rs",
        [6, 0, 0],
        "`&mut self` in `impl EcsMaster`: no worker holds one (R2', R4, G-a)",
    ),
    (
        "ecs/core/ecs_master/ecs_master.rs",
        [2, 1, 0],
        "`&mut self` in `impl EcsMaster`; the drain asserts `!is_in_system_run()`; one S1 is \
         `Box::leak(cell)`, not a world pointer",
    ),
    ("ecs/core/bundle/self_bundle.rs", [0, 0, 1], "not a world pointer: `&raw const *this`"),
];

/// R4: the `.world_mut()` call lines in non-test `src/**`, per file, each with its solo gate.
const R4_ALLOW: [(&str, usize, &str); 2] = [
    (
        "ecs/core/schedule/schedule.rs",
        4,
        "dispatcher-solo: apply-window drain (`pending == running || running == 0`), \
         conditions (`running == 0`), EXC2 inline exclusive + its apply",
    ),
    (
        "ecs/core/system/exclusive_function_system.rs",
        1,
        "dispatcher-solo: reached only through `run_dispatcher`",
    ),
];

/// One rule violation, with where and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Red {
    rule: &'static str,
    file: String,
    line: usize,
    msg: String,
}

impl std::fmt::Display for Red {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RED [{}] {}:{}: {}", self.rule, self.file, self.line, self.msg)
    }
}

fn red(rule: &'static str, file: &str, line: usize, msg: String) -> Red {
    Red { rule, file: file.to_owned(), line, msg }
}

// ─── Lexing: comments out, line structure kept ────────────────────────────────

/// Returns `text` with every comment (line, doc and nested block) replaced by spaces, string and
/// char literal contents kept verbatim, and every newline kept, so line numbers survive.
fn strip_comments(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(if b[i] == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && !prev_is_ident(&b, i) {
            // Raw string `r"…"` / `r#"…"#`: copied verbatim to its matching close.
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&'"') {
                let start = i;
                j += 1;
                loop {
                    if j >= b.len() {
                        break;
                    }
                    if b[j] == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                out.extend(&b[start..j.min(b.len())]);
                i = j;
            } else {
                out.push(c);
                i += 1;
            }
        } else if c == '"' {
            out.push(c);
            i += 1;
            while i < b.len() {
                out.push(b[i]);
                if b[i] == '\\' {
                    if let Some(&e) = b.get(i + 1) {
                        out.push(e);
                    }
                    i += 2;
                    continue;
                }
                i += 1;
                if b[i - 1] == '"' {
                    break;
                }
            }
        } else if c == '\'' {
            // A char literal is `'x'` or `'\…'`; anything else is a lifetime or label.
            if next == Some('\\') {
                // Skip the escaped character itself, so `'\''` closes on its third quote.
                let mut j = i + 3;
                while j < b.len() && b[j] != '\'' {
                    j += 1;
                }
                out.extend(&b[i..(j + 1).min(b.len())]);
                i = j + 1;
            } else if b.get(i + 2) == Some(&'\'') {
                out.extend(&b[i..i + 3]);
                i += 3;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn prev_is_ident(b: &[char], i: usize) -> bool {
    i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_')
}

/// `(1-based line number, code)` for every non-blank line of the non-test part of `text`,
/// comments removed. The file is cut at the first `#[cfg(test)]` that applies to a `mod … {`
/// (blank lines and further attributes may sit between the two).
fn nontest_code(text: &str) -> Vec<(usize, String)> {
    let stripped = strip_comments(text);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(tail) = line.trim().strip_prefix("#[cfg(test)]") {
            let inline_here = mod_decl(tail).is_some_and(|(_, inline)| inline);
            if inline_here || (tail.trim().is_empty() && next_item_is_inline_mod(&lines[i + 1..])) {
                break;
            }
        }
        if !line.trim().is_empty() {
            out.push((i + 1, (*line).to_owned()));
        }
    }
    out
}

/// The item after an attribute: the first line that is neither blank nor another attribute.
fn next_item<'a>(rest: &[&'a str]) -> Option<&'a str> {
    rest.iter().map(|l| l.trim()).find(|l| !l.is_empty() && !l.starts_with("#["))
}

fn mod_decl(line: &str) -> Option<(&str, bool)> {
    let mut s = line.trim();
    if let Some(r) = s.strip_prefix("pub") {
        s = r.trim_start();
        if s.starts_with('(') {
            s = &s[s.find(')')? + 1..];
        }
        s = s.trim_start();
    }
    let r = s.strip_prefix("mod ")?.trim_start();
    let end = r.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let (name, tail) = r.split_at(end);
    let tail = tail.trim_start();
    if tail.starts_with('{') {
        Some((name, true))
    } else if tail.starts_with(';') {
        Some((name, false))
    } else {
        None
    }
}

fn next_item_is_inline_mod(rest: &[&str]) -> bool {
    next_item(rest).and_then(mod_decl).is_some_and(|(_, inline)| inline)
}

/// Files declared as `#[cfg(test)] mod x;` somewhere in `files` (paths relative to `src/`).
fn test_module_files(files: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (rel, text) in files {
        let stripped = strip_comments(text);
        let lines: Vec<&str> = stripped.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(tail) = line.trim().strip_prefix("#[cfg(test)]") else {
                continue;
            };
            let decl = if tail.trim().is_empty() {
                next_item(&lines[i + 1..]).and_then(mod_decl)
            } else {
                mod_decl(tail)
            };
            let Some((name, false)) = decl else {
                continue;
            };
            let dir = match rel.rsplit_once('/') {
                Some((d, f)) if f == "mod.rs" || f == "lib.rs" => d.to_owned(),
                Some((d, f)) => format!("{d}/{}", f.trim_end_matches(".rs")),
                None if rel == "lib.rs" => String::new(),
                None => rel.trim_end_matches(".rs").to_owned(),
            };
            let base = if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
            out.insert(format!("{base}.rs"));
            out.insert(format!("{base}/"));
        }
    }
    out
}

fn is_excluded(rel: &str, excluded: &BTreeSet<String>) -> bool {
    excluded.contains(rel) || excluded.iter().any(|e| e.ends_with('/') && rel.starts_with(e.as_str()))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_word(line: &str, word: &str) -> usize {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    line.match_indices(word)
        .filter(|&(at, _)| {
            let before = at.checked_sub(1).map(|k| bytes[k]);
            let after = bytes.get(at + word.len()).copied();
            !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
        })
        .count()
}

// ─── R2′: the cell's private `ptr` field ───────────────────────────────────────

/// (line, enclosing item, collapsed line) for every `ptr` token in the non-test part of `text`.
fn r2a_rows(text: &str) -> Vec<(usize, String, String)> {
    let mut item = String::from("<file>");
    let mut rows = Vec::new();
    for (n, code) in nontest_code(text) {
        if let Some(name) = fn_or_cell_struct(&code) {
            item = name;
        }
        for _ in 0..has_word(&code, "ptr") {
            rows.push((n, item.clone(), collapse(&code)));
        }
    }
    rows
}

fn fn_or_cell_struct(code: &str) -> Option<String> {
    if has_word(code, "struct") > 0 && has_word(code, "UnsafeEcsCell") > 0 {
        return Some("struct UnsafeEcsCell".to_owned());
    }
    let at = code.match_indices("fn ").find(|&(at, _)| {
        at == 0 || !code.as_bytes()[at - 1].is_ascii_alphanumeric() && code.as_bytes()[at - 1] != b'_'
    })?;
    let rest = code[at.0 + 3..].trim_start();
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
}

fn check_r2a(file: &str, text: &str, allow: &[(&str, &str)]) -> Vec<Red> {
    let mut want: Vec<(String, String)> =
        allow.iter().map(|&(i, l)| (i.to_owned(), collapse(l))).collect();
    let mut reds = Vec::new();
    for (n, item, line) in r2a_rows(text) {
        if let Some(k) = want.iter().position(|w| w.0 == item && w.1 == line) {
            want.swap_remove(k);
        } else {
            reds.push(red(
                "R2'(a) new row",
                file,
                n,
                format!(
                    "`ptr` (the cell's private world pointer) used in `{item}`: `{line}`. Every \
                     shape that reaches the world through the cell is reviewed: a field projection \
                     followed by a `&mut self` method, an alias or a destructure can re-form the \
                     worker-side `&mut` PC-24 removed (W0). Add a row only with its reason."
                ),
            ));
        }
    }
    for (item, line) in want {
        reds.push(red(
            "R2'(a) missing row",
            file,
            0,
            format!(
                "allowlisted `{item}`: `{line}` no longer occurs; a deleted row is never a \
                 smaller green — update the allowlist in the same change"
            ),
        ));
    }
    reds
}

fn check_r2b(file: &str, text: &str) -> Vec<Red> {
    nontest_code(text)
        .into_iter()
        .filter(|(_, code)| code.trim_start().starts_with("#[path") || mod_decl(code).is_some())
        .map(|(n, code)| {
            red(
                "R2'(b)",
                file,
                n,
                format!(
                    "module declaration in the non-test part of the cell file: `{}` — a child \
                     module can name the private `ptr` field from another file, outside R2'(a)",
                    collapse(&code)
                ),
            )
        })
        .collect()
}

// ─── R1′: raw-world-pointer mint census ────────────────────────────────────────

/// `[S1, S2, S5, guard]` counts over the non-test code of `text`.
fn mint_counts(text: &str) -> [usize; 4] {
    let mut c = [0usize; 4];
    for (_, code) in nontest_code(text) {
        c[0] += code.matches("NonNull::from(").count();
        c[1] += code.matches("*mut EcsMaster").count()
            + code.matches("*const EcsMaster").count()
            + code.matches("NonNull<EcsMaster>").count();
        c[2] += ["&raw const *", "&raw mut *", "&raw const (*", "&raw mut (*"]
            .iter()
            .map(|p| code.matches(p).count())
            .sum::<usize>();
        c[3] += ["as *mut Self", "as *const Self"]
            .iter()
            .map(|p| {
                code.match_indices(p)
                    .filter(|&(at, _)| {
                        !code.as_bytes().get(at + p.len()).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    })
                    .count()
            })
            .sum::<usize>();
    }
    c
}

fn check_r1(files: &BTreeMap<String, String>, allow: &[(&str, [usize; 3], &str)]) -> Vec<Red> {
    let excluded = test_module_files(files);
    let mut reds = Vec::new();
    let mut seen = BTreeSet::new();
    for (rel, text) in files {
        if is_excluded(rel, &excluded) {
            continue;
        }
        let c = mint_counts(text);
        if c[3] != 0 {
            reds.push(red(
                "R1' guard",
                rel,
                0,
                format!("{} `as *mut/*const Self` cast(s): a world-pointer mint the census must see", c[3]),
            ));
        }
        let got = [c[0], c[1], c[2]];
        match allow.iter().find(|a| a.0 == rel) {
            Some(&(_, want, _)) => {
                seen.insert(rel.clone());
                if got != want {
                    reds.push(red(
                        "R1' count",
                        rel,
                        0,
                        format!(
                            "[NonNull::from(, *mut/*const EcsMaster|NonNull<EcsMaster>, &raw *] = \
                             {got:?}, allowlisted {want:?}; a new mint needs its reason, a \
                             removed one its row updated"
                        ),
                    ));
                }
            }
            None if got != [0, 0, 0] => reds.push(red(
                "R1' new file",
                rel,
                0,
                format!(
                    "names or mints a raw world pointer {got:?} and is not in the allowlist: \
                     write why it is not on a parallel path (dispatcher / apply window / \
                     `&mut self`) before adding it"
                ),
            )),
            None => {}
        }
    }
    for &(rel, _, _) in allow {
        if !seen.contains(rel) {
            reds.push(red(
                "R1' missing row",
                rel,
                0,
                "allowlisted file absent or excluded; a deleted row is never a smaller green".to_owned(),
            ));
        }
    }
    reds
}

// ─── R4 / R5: `.world_mut()` call sites ────────────────────────────────────────

fn world_mut_lines(text: &str) -> Vec<usize> {
    nontest_code(text)
        .into_iter()
        .flat_map(|(n, code)| std::iter::repeat_n(n, code.matches(".world_mut()").count()))
        .collect()
}

fn check_r4(files: &BTreeMap<String, String>, allow: &[(&str, usize, &str)]) -> Vec<Red> {
    let excluded = test_module_files(files);
    let mut reds = Vec::new();
    let mut seen = BTreeSet::new();
    for (rel, text) in files {
        if is_excluded(rel, &excluded) {
            continue;
        }
        let lines = world_mut_lines(text);
        let want = allow.iter().find(|a| a.0 == rel).map(|a| a.1);
        if want.is_some() {
            seen.insert(rel.clone());
        }
        if lines.len() != want.unwrap_or(0) {
            reds.push(red(
                "R4",
                rel,
                lines.first().copied().unwrap_or(0),
                format!(
                    "{} `.world_mut()` call(s) at lines {lines:?}, allowlisted {}: a whole-world \
                     `&mut` may be formed only where no worker is live — name the solo gate",
                    lines.len(),
                    want.unwrap_or(0)
                ),
            ));
        }
    }
    for &(rel, _, _) in allow {
        if !seen.contains(rel) {
            reds.push(red(
                "R4 missing row",
                rel,
                0,
                "allowlisted file absent or excluded; a deleted row is never a smaller green".to_owned(),
            ));
        }
    }
    reds
}

fn check_r5(file: &str, text: &str) -> Vec<Red> {
    world_mut_lines(text)
        .into_iter()
        .map(|n| {
            red(
                "R5",
                file,
                n,
                "`.world_mut()` inside the cell's non-test code: the PC-24 shape — a worker-side \
                 accessor re-forming `&mut EcsMaster` (use a shared projection, W0)"
                    .to_owned(),
            )
        })
        .collect()
}

// ─── The live tree ─────────────────────────────────────────────────────────────

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read_tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            .map(|e| e.expect("invariant: a readable directory entry").path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(root)
                    .expect("invariant: walked under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = fs::read_to_string(&p)
                    .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
                out.insert(rel, text);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn fail_on(reds: &[Red]) {
    assert!(
        reds.is_empty(),
        "world-reference census RED ({}):\n{}",
        reds.len(),
        reds.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn world_ref_census_live_tree() {
    let files = read_tree(&src_root());
    assert!(
        files.len() > 100,
        "the scan visited {} .rs files under src/; an empty or truncated glob cannot pass",
        files.len()
    );
    let cell = files.get(CELL).unwrap_or_else(|| panic!("the scan did not visit {CELL}"));

    // The live-scan guards join the rule REDs, so one run reports every failure at once.
    let mut reds = Vec::new();
    let rows = r2a_rows(cell).len();
    if rows != R2A_ALLOW.len() {
        reds.push(red(
            "R2'(a) live rows",
            CELL,
            0,
            format!("{rows} `ptr` rows, allowlisted {}", R2A_ALLOW.len()),
        ));
    }
    let excluded = test_module_files(&files);
    let minting = files
        .iter()
        .filter(|(rel, text)| !is_excluded(rel, &excluded) && mint_counts(text)[..3] != [0, 0, 0])
        .count();
    if minting != R1_ALLOW.len() {
        reds.push(red(
            "R1' live files",
            "src/**",
            0,
            format!("{minting} files name or mint a raw world pointer, allowlisted {}", R1_ALLOW.len()),
        ));
    }

    reds.extend(check_r2a(CELL, cell, &R2A_ALLOW));
    reds.extend(check_r2b(CELL, cell));
    reds.extend(check_r1(&files, &R1_ALLOW));
    reds.extend(check_r4(&files, &R4_ALLOW));
    reds.extend(check_r5(CELL, cell));
    fail_on(&reds);
}

// ─── Fixtures: every rule must be able to RED ────────────────────────────────────

/// A cell-shaped fixture: `body` placed inside `fn <name>` inside an `impl`, followed by a
/// test module whose content must be ignored.
fn cell_fixture(name: &str, body: &str) -> String {
    format!(
        "impl<'w> UnsafeEcsCell<'w> {{\n    pub(crate) unsafe fn {name}(self, id: ArchetypeId) {{\n{body}\n    }}\n}}\n\n#[cfg(test)]\nmod tests {{\n    fn t() {{ let p = cell.ptr; cell.world_mut(); }}\n}}\n"
    )
}

fn assert_red_at(reds: &[Red], rule: &str, line: usize) {
    assert!(
        reds.iter().any(|r| r.rule == rule && r.line == line),
        "expected RED [{rule}] at line {line}, got:\n{}",
        reds.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
    );
}

fn new_rows(reds: &[Red]) -> Vec<usize> {
    reds.iter().filter(|r| r.rule == "R2'(a) new row").map(|r| r.line).collect()
}

/// FX1 — trunk `unsafe_ecs_cell.rs:218`, the PC-24 line itself: R5 and R4 RED.
#[test]
fn fx1_trunk_archetype_ptr_mut_reds_r5_and_r4() {
    let text = cell_fixture(
        "archetype_ptr_mut",
        "        unsafe { self.world_mut().archetype_master_mut().archetype_ptr_for(id) }",
    );
    assert_red_at(&check_r5(CELL, &text), "R5", 3);
    let files = BTreeMap::from([(CELL.to_owned(), text)]);
    assert_red_at(&check_r4(&files, &R4_ALLOW), "R4", 3);
}

/// FX2 — the naive rewrite of `:218` (project the field, then call the `&mut self` method):
/// invisible to R4/R5 and to the tripwire, caught by R2′(a) as a new row.
#[test]
fn fx2_naive_field_projection_rewrite_reds_r2a() {
    let text = cell_fixture(
        "archetype_ptr_mut",
        "        unsafe { (*self.ptr).archetype_master.archetype_ptr_for(id) }",
    );
    assert_eq!(new_rows(&check_r2a(CELL, &text, &R2A_ALLOW)), vec![3]);
    assert!(check_r5(CELL, &text).is_empty(), "no `.world_mut()` in the naive rewrite");
}

/// FX3 — any method through the `entity_master` field projection.
#[test]
fn fx3_entity_master_method_reds_r2a() {
    let text = cell_fixture(
        "entity_counter",
        "        unsafe { (*self.ptr).entity_master.reserve_entity() }",
    );
    assert_eq!(new_rows(&check_r2a(CELL, &text, &R2A_ALLOW)), vec![3]);
}

/// FX4 — trunk `:369`, the deleted `resources_mut`: a `&mut Resources` on a worker.
#[test]
fn fx4_trunk_resources_mut_reds_r2a() {
    let text = cell_fixture("resources_mut", "        unsafe { &mut (*self.ptr).resources }");
    assert_eq!(new_rows(&check_r2a(CELL, &text, &R2A_ALLOW)), vec![3]);
}

/// FX5 / FX6 / FX7 — an alias, a split projection and a destructure of the private field.
#[test]
fn fx5_fx6_fx7_alias_split_destructure_red_r2a() {
    let alias = cell_fixture("world", "        let p = self.ptr;");
    assert_eq!(new_rows(&check_r2a(CELL, &alias, &R2A_ALLOW)), vec![3]);
    let split = cell_fixture("resources", "        unsafe { &mut (*self\n            .ptr).resources }");
    assert_eq!(new_rows(&check_r2a(CELL, &split, &R2A_ALLOW)), vec![4]);
    let destructure = cell_fixture("world", "        let Self { ptr, .. } = self;");
    assert_eq!(new_rows(&check_r2a(CELL, &destructure, &R2A_ALLOW)), vec![3]);
}

/// FX8 — a `#[path]` child module in the non-test part of the cell file.
#[test]
fn fx8_path_module_reds_r2b() {
    let text = format!("#[path = \"x.rs\"]\nmod x;\n\n{}", cell_fixture("world", "        let _ = 0;"));
    let reds = check_r2b(CELL, &text);
    assert_red_at(&reds, "R2'(b)", 1);
    assert_red_at(&reds, "R2'(b)", 2);
}

/// FX9 — a new file minting a raw world pointer with an inferred type.
#[test]
fn fx9_new_minting_file_reds_r1() {
    let files = BTreeMap::from([(
        "ecs/core/new_worker_path.rs".to_owned(),
        "fn f(world: &mut EcsMaster) {\n    let world_ptr = NonNull::from(&mut *world);\n}\n".to_owned(),
    )]);
    let reds = check_r1(&files, &R1_ALLOW);
    assert!(
        reds.iter().any(|r| r.rule == "R1' new file" && r.file == "ecs/core/new_worker_path.rs"),
        "{reds:#?}"
    );
}

/// FX10 — a new `.world_mut()` call site REDs R4; one inside a file declared
/// `#[cfg(test)] mod x;` is not counted.
#[test]
fn fx10_new_world_mut_site_reds_r4_and_test_module_file_is_skipped() {
    let files = BTreeMap::from([
        (
            "ecs/core/new_site.rs".to_owned(),
            "fn f(cell: UnsafeEcsCell<'_>) {\n    let w = unsafe { cell.world_mut() };\n}\n".to_owned(),
        ),
        ("ecs/core/prof/mod.rs".to_owned(), "pub mod real;\n\n#[cfg(test)]\nmod tests;\n".to_owned()),
        ("ecs/core/prof/tests.rs".to_owned(), "fn t() {\n    app.world_mut().run();\n}\n".to_owned()),
    ]);
    let reds = check_r4(&files, &R4_ALLOW);
    assert_red_at(&reds, "R4", 2);
    assert!(reds.iter().all(|r| r.file != "ecs/core/prof/tests.rs"), "{reds:#?}");
    assert!(test_module_files(&files).contains("ecs/core/prof/tests.rs"));
}

/// FX-clean — the lane's own cell file, read at test time: GREEN on R2′, R4 and R5, and the
/// fixture cell's trailing test module is ignored by every rule.
#[test]
fn fx_clean_live_cell_is_green() {
    let cell = fs::read_to_string(src_root().join(CELL)).expect("invariant: the cell file exists");
    let mut reds = check_r2a(CELL, &cell, &R2A_ALLOW);
    reds.extend(check_r2b(CELL, &cell));
    reds.extend(check_r5(CELL, &cell));
    let files = BTreeMap::from([(CELL.to_owned(), cell)]);
    reds.extend(check_r4(&files, &R4_ALLOW).into_iter().filter(|r| r.file == CELL));
    fail_on(&reds);

    let fixture = cell_fixture("world", "        unsafe { &*self.ptr }");
    assert!(new_rows(&check_r2a(CELL, &fixture, &R2A_ALLOW)).is_empty());
    assert!(check_r5(CELL, &fixture).is_empty(), "the fixture's test module must be cut");
}

/// The test-module cut: through a second attribute and a comment (`app.rs`'s shape), on one
/// line, and never at a `#[cfg(test)]` on a non-module item.
#[test]
fn nontest_cut_sees_through_attributes_and_ignores_test_items() {
    let app_shape = "fn a() { let x = &raw const *field; }\n\n#[cfg(test)]\n#[cfg(not(miri))] // real pool\nmod tests {\n    fn t() { let y = &raw const *field; }\n}\n";
    assert_eq!(mint_counts(app_shape)[2], 1, "only the line before the test module counts");
    let one_line = "fn a() {}\n#[cfg(test)] mod tests { fn t() { cell.world_mut(); } }\n";
    assert!(world_mut_lines(one_line).is_empty());
    let test_item = "#[cfg(test)]\nfn helper() {}\nfn b(cell: C) { cell.world_mut(); }\n";
    assert_eq!(world_mut_lines(test_item), vec![3], "a cfg(test) fn does not cut the file");
}

/// The lexer the rules stand on: comments go, strings and line numbers stay.
#[test]
fn strip_comments_keeps_code_strings_and_lines() {
    let text = "let a = 1; // self.ptr\n/* self.ptr\n */ let b = \"// not a comment\";\n/// doc .world_mut()\nlet c = '\"'; let d = self.ptr;\n";
    let s = strip_comments(text);
    assert_eq!(s.lines().count(), text.lines().count());
    assert!(!s.lines().next().expect("line 1").contains("ptr"));
    assert!(s.contains("\"// not a comment\""));
    assert!(!s.contains("world_mut"));
    assert_eq!(has_word(s.lines().nth(4).expect("line 5"), "ptr"), 1);
}
