//! **Reflection GATES G1 — the manifest census: `reflect` features are leaves (P-A).**
//!
//! Asserts `docs/REFLECTION-PLAN-GATES.md` D3's six clauses over every workspace member,
//! reading `cargo metadata --format-version 1 --no-deps`'s **packages** half — the
//! manifests as written, which is invocation-independent. Every clause is decidable
//! without resolving anything: each dependency entry carries `kind` / `optional` /
//! `features` / `uses_default_features`, and each package carries its own `features` map.
//!
//! # Why this clause set and not "just run `cargo tree`"
//!
//! C1/C4/C5 are the only instruments in the reflection plan that see the §2 marker-feature
//! footgun **before** it fires. `cargo tree` reports a closure; these report that no
//! closure *could* contain `boyko-reflect`. The measurement that makes them necessary is
//! in this tree already: while `profiling-analysis` was default-on, no command line could
//! turn it off, because unification restored it through nine sibling manifests
//! (`crates/boyko_ecs/Cargo.toml`). A gate that only reads closures would have reported
//! that configuration green until someone tried the command line.
//!
//! # Why it lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point
//! the scan at the wrong tree — `internal_docs_anchors.rs` / `engine_packages_census.rs`'s
//! rationale, verbatim. The root package has effectively no dependencies, so this gate
//! needs no engine build.
//!
//! # RED ledger (GATES Appendix GB, G1 rows — run at landing, 2026-08-21)
//!
//! * R1a `default = ["reflect"]` on `reflect-fixture` ⇒ C1 reds naming the member.
//! * R1b `boyko-reflect` edge made non-optional ⇒ C2 reds.
//! * R1c `reflect = ["boyko-reflect"]` (no `dep:`) ⇒ C3 reds.
//! * R1d `features = ["reflect"]` on `reflect-dogfood`'s `boyko-scene` edge ⇒ C4 reds.
//! * R1e `reflect = ["boyko-render/reflect"]` on `boyko_demo` ⇒ C5 reds naming the ship
//!   target.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::sync::OnceLock;

/// The C6 scanner, shared with `tests/reflect_ci_coverage.rs` so the clause's two
/// halves (found ⊆ named here; named == found there, at G4) cannot drift apart.
#[path = "reflect_scan_support/mod.rs"]
mod support;

use support::{cross_entry, named_specs, repo_root, scan_scope};

// ── The decision the failure messages cite (GATES G1: "the failure message names the
//    decision, not only the clause") ─────────────────────────────────────────────────
const B12_RULE: &str = "engine crates MAY declare `reflect` (`REFLECTION-ANALYSIS.md` B.12); \
     what they may not do is let anything enable it -- this is that rule, not the older leaf rule";

/// The ship-target members (GATES D2 is the single owner of this list): the only
/// game-shaped `[[bin]]` member, and the workspace root package. **Not `boyko_app`** —
/// it is `[lib]`-only and has no artifact to census.
const SHIP_TARGETS: &[&str] = &["boyko_demo", "boyko-engine"];

/// The package under gate. Spelled as `cargo metadata` prints it (hyphenated — this
/// workspace does not spell members uniformly, and `engine_packages_census` exists
/// because a campaign once assumed it did).
const REFLECT_CRATE: &str = "boyko-reflect";

// ─────────────────────────────────────────────────────────────────────────────────────
// A minimal JSON reader for `cargo metadata` output.
//
// Deliberately hand-rolled: the root package's dev-dependency table is `boyko-diag`
// alone, and pulling serde/serde_json for one census would put a third-party graph
// under a gate whose whole subject is dependency hygiene. ~120 lines, objects as
// key-ordered pair vectors (no `HashMap` — the workspace type ban applies to tests the
// same way `clippy.toml` applies it everywhere).
// ─────────────────────────────────────────────────────────────────────────────────────

enum Json {
    Object(Vec<(String, Json)>),
    Array(Vec<Json>),
    Str(String),
    Bool(bool),
    /// Payload-free: the census reads no numeric field, but a number must still parse so
    /// the reader does not silently desynchronise from the document.
    Num,
    Null,
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => panic!("census internal error: expected a JSON array"),
        }
    }
    fn as_str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("census internal error: expected a JSON string"),
        }
    }
    fn as_bool(&self) -> bool {
        match self {
            Json::Bool(b) => *b,
            _ => panic!("census internal error: expected a JSON bool"),
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser { bytes: text.as_bytes(), pos: 0 }
    }

    fn fail(&self, what: &str) -> ! {
        panic!("census internal error: malformed cargo metadata JSON at byte {}: {what}", self.pos)
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> u8 {
        *self.bytes.get(self.pos).unwrap_or(&0)
    }

    fn expect(&mut self, b: u8) {
        if self.peek() != b {
            self.fail(&format!("expected `{}`", b as char));
        }
        self.pos += 1;
    }

    fn value(&mut self) -> Json {
        self.skip_ws();
        match self.peek() {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Json::Str(self.string()),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => self.fail("unexpected byte at start of value"),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Json {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            value
        } else {
            self.fail("bad literal")
        }
    }

    fn number(&mut self) -> Json {
        let start = self.pos;
        while matches!(self.peek(), b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("");
        if text.parse::<f64>().is_err() {
            self.fail("bad number");
        }
        Json::Num
    }

    fn string(&mut self) -> String {
        self.expect(b'"');
        let mut out = String::new();
        loop {
            match self.peek() {
                b'"' => {
                    self.pos += 1;
                    return out;
                }
                b'\\' => {
                    self.pos += 1;
                    let esc = self.peek();
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4();
                            let cp = if (0xD800..0xDC00).contains(&hi)
                                && self.bytes[self.pos..].starts_with(b"\\u")
                            {
                                self.pos += 2;
                                let lo = self.hex4();
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => self.fail("bad escape"),
                    }
                }
                0 => self.fail("unterminated string"),
                _ => {
                    // Multi-byte UTF-8 passes through byte-wise; the source is a valid
                    // `String`, so slicing at the next `"` or `\\` is safe.
                    let start = self.pos;
                    while !matches!(self.peek(), b'"' | b'\\' | 0) {
                        self.pos += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or_else(
                        |_| panic!("census internal error: non-UTF-8 in metadata string"),
                    ));
                }
            }
        }
    }

    fn hex4(&mut self) -> u32 {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = (self.peek() as char).to_digit(16).unwrap_or_else(|| self.fail("bad \\u escape"));
            v = v * 16 + d;
            self.pos += 1;
        }
        v
    }

    fn object(&mut self) -> Json {
        self.expect(b'{');
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == b'}' {
            self.pos += 1;
            return Json::Object(pairs);
        }
        loop {
            self.skip_ws();
            let key = self.string();
            self.skip_ws();
            self.expect(b':');
            let value = self.value();
            pairs.push((key, value));
            self.skip_ws();
            match self.peek() {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    return Json::Object(pairs);
                }
                _ => self.fail("expected `,` or `}` in object"),
            }
        }
    }

    fn array(&mut self) -> Json {
        self.expect(b'[');
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == b']' {
            self.pos += 1;
            return Json::Array(items);
        }
        loop {
            items.push(self.value());
            self.skip_ws();
            match self.peek() {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    return Json::Array(items);
                }
                _ => self.fail("expected `,` or `]` in array"),
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// The manifest model the clauses run over.
// ─────────────────────────────────────────────────────────────────────────────────────

/// One dependency edge, as the manifest wrote it.
struct Dep {
    /// The dependency's PACKAGE name (renames keep this — `rename` only changes the key
    /// the consumer types, so a renamed `boyko-reflect` edge is still caught here).
    name: String,
    /// `null` (normal) / `"dev"` / `"build"`.
    kind: Option<String>,
    optional: bool,
    /// The edge's `features = [...]` array — C4's subject.
    features: Vec<String>,
}

/// One workspace member's manifest, reduced to what D3's clauses read.
struct Member {
    name: String,
    /// `[features]` as written: name → entry list.
    features: Vec<(String, Vec<String>)>,
    deps: Vec<Dep>,
}

impl Member {
    fn feature(&self, name: &str) -> Option<&[String]> {
        self.features.iter().find(|(n, _)| n == name).map(|(_, l)| l.as_slice())
    }
}

/// Runs `cargo metadata --format-version 1 --no-deps` at the repository root and reduces
/// its packages half. Tool failure is a panic, never a skip (GATES D6).
fn members() -> &'static BTreeMap<String, Member> {
    static MEMBERS: OnceLock<BTreeMap<String, Member>> = OnceLock::new();
    MEMBERS.get_or_init(|| {
        let out = Command::new(env!("CARGO"))
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .current_dir(repo_root())
            .output()
            .unwrap_or_else(|e| panic!("could not spawn cargo metadata: {e}"));
        assert!(
            out.status.success(),
            "cargo metadata failed, so the manifest census has no subject:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout)
            .unwrap_or_else(|e| panic!("cargo metadata emitted non-UTF-8: {e}"));
        let root = Parser::new(&text).value();

        let mut map = BTreeMap::new();
        for pkg in root.get("packages").expect("metadata has a packages array").as_array() {
            let name = pkg.get("name").expect("package has a name").as_str().to_owned();
            let mut features = Vec::new();
            if let Some(Json::Object(pairs)) = pkg.get("features") {
                for (fname, list) in pairs {
                    let entries = list.as_array().iter().map(|e| e.as_str().to_owned()).collect();
                    features.push((fname.clone(), entries));
                }
            }
            let mut deps = Vec::new();
            for dep in pkg.get("dependencies").expect("package has dependencies").as_array() {
                deps.push(Dep {
                    name: dep.get("name").expect("dep has a name").as_str().to_owned(),
                    kind: match dep.get("kind") {
                        Some(Json::Str(s)) => Some(s.clone()),
                        _ => None,
                    },
                    optional: dep.get("optional").expect("dep has optional").as_bool(),
                    features: dep
                        .get("features")
                        .expect("dep has features")
                        .as_array()
                        .iter()
                        .map(|f| f.as_str().to_owned())
                        .collect(),
                });
            }
            map.insert(name.clone(), Member { name, features, deps });
        }
        assert!(
            map.len() > 20,
            "cargo metadata reported only {} workspace members -- the census is scanning \
             the wrong tree",
            map.len()
        );
        map
    })
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C1 — never default.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C1**: no member's `default` list transitively enables a `reflect` feature, and no
/// member's `default` names `boyko-reflect`. The expansion crosses members (`pkg/feat`
/// entries recurse into the named member's own feature table), because that is exactly
/// how `profiling-analysis` came back through nine sibling manifests.
#[test]
fn c1_no_default_reaches_reflect() {
    let members = members();
    let mut violations = Vec::new();

    for member in members.values() {
        if member.feature("default").is_none() {
            continue;
        }
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut stack = vec![(member.name.clone(), String::from("default"))];
        while let Some((pkg, feat)) = stack.pop() {
            if !seen.insert((pkg.clone(), feat.clone())) {
                continue;
            }
            if feat == "reflect" {
                violations.push(format!(
                    "{}: `default` transitively enables `{pkg}/reflect`",
                    member.name
                ));
                continue;
            }
            let Some(list) = members.get(&pkg).and_then(|m| m.feature(&feat)) else {
                continue;
            };
            for entry in list {
                if let Some(dep) = entry.strip_prefix("dep:") {
                    if dep == REFLECT_CRATE {
                        violations.push(format!(
                            "{}: `default` transitively pulls `dep:{REFLECT_CRATE}` (via \
                             `{pkg}/{feat}`)",
                            member.name
                        ));
                    }
                } else if let Some((dpkg, _weak, dfeat)) = cross_entry(entry) {
                    if dpkg == REFLECT_CRATE {
                        violations.push(format!(
                            "{}: `default` transitively pulls `{entry}` (via `{pkg}/{feat}`)",
                            member.name
                        ));
                    }
                    if members.contains_key(dpkg) {
                        stack.push((dpkg.to_owned(), dfeat.to_owned()));
                    }
                } else if entry == REFLECT_CRATE {
                    violations.push(format!(
                        "{}: `default` names `{REFLECT_CRATE}` bare (via `{pkg}/{feat}`)",
                        member.name
                    ));
                } else {
                    stack.push((pkg.clone(), entry.clone()));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "C1 (never default) is violated -- this is the `profiling-analysis` failure, \
         mechanically caught: a default-on feature has NO command line that can turn it off, \
         because unification restores it through every sibling manifest that depends without \
         `default-features = false`:\n  {}",
        violations.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C2 — optional edges only.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C2**: every dependency edge naming `boyko-reflect` has `optional == true`. A
/// non-optional edge is unconditionally in the consumer's closure — the feature stops
/// being an opt-in and becomes a label.
#[test]
fn c2_every_boyko_reflect_edge_is_optional() {
    let mut violations = Vec::new();
    for member in members().values() {
        for dep in &member.deps {
            if dep.name == REFLECT_CRATE && !dep.optional {
                violations.push(format!(
                    "{} -> {REFLECT_CRATE} ({} edge) is NOT optional",
                    member.name,
                    dep.kind.as_deref().unwrap_or("normal"),
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "C2 (optional edges only) is violated -- a non-optional `{REFLECT_CRATE}` edge puts \
         the editor's reflection layer in that consumer's closure UNCONDITIONALLY, and no \
         feature selection can remove it:\n  {}",
        violations.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C3 — `dep:` discipline.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C3**: a feature that pulls the crate is written exactly `["dep:boyko-reflect", …]`.
/// The bare `["boyko-reflect"]` form implicitly mints a SECOND, differently-named feature
/// — the way a consumer silently acquires an always-on optional dependency.
#[test]
fn c3_reflect_is_pulled_only_through_dep_syntax() {
    let mut violations = Vec::new();
    for member in members().values() {
        for (fname, entries) in &member.features {
            for entry in entries {
                let names_reflect = entry == REFLECT_CRATE
                    || cross_entry(entry).is_some_and(|(p, _, _)| p == REFLECT_CRATE);
                if names_reflect {
                    violations.push(format!(
                        "{}: feature `{fname}` contains `{entry}` -- the only permitted \
                         form is `dep:{REFLECT_CRATE}`",
                        member.name
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "C3 (`dep:` discipline) is violated -- a bare `{REFLECT_CRATE}` feature entry \
         implicitly creates a second feature named after the dependency, and that implicit \
         feature is how a consumer silently gets an always-on optional dep:\n  {}",
        violations.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C4 — no edge enables it.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C4**: no dependency edge of ANY kind lists `reflect` (or a `…/reflect` forward) in
/// its `features` array. An edge is the one enabler nothing can switch off:
/// `crates/boyko_ecs/Cargo.toml` records that *"an explicit `features = [...]` survives
/// `--no-default-features` by design"*.
#[test]
fn c4_no_dependency_edge_enables_reflect() {
    let mut violations = Vec::new();
    for member in members().values() {
        for dep in &member.deps {
            for feat in &dep.features {
                let enables = feat == "reflect"
                    || cross_entry(feat).is_some_and(|(_, _, f)| f == "reflect");
                if enables {
                    violations.push(format!(
                        "{} -> {} ({} edge) carries `features = [.. \"{feat}\" ..]`",
                        member.name,
                        dep.name,
                        dep.kind.as_deref().unwrap_or("normal"),
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "C4 (no edge enables it) is violated -- an edge's `features = [...]` survives \
         `--no-default-features` by design, so this is the one enabling form nothing can \
         switch off. Enablement is by a `[features]` forward or a command line, never by an \
         edge. {B12_RULE}:\n  {}",
        violations.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C5 — ship targets are clean.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C5**: no ship-target member declares OR forwards a `reflect` feature. This is what
/// survives of the old leaf rule, narrowed to where unification can actually reach a
/// shipped artifact.
#[test]
fn c5_ship_targets_declare_and_forward_nothing() {
    let members = members();
    let mut violations = Vec::new();
    for ship in SHIP_TARGETS {
        let member = members.get(*ship).unwrap_or_else(|| {
            panic!(
                "ship target `{ship}` is not a workspace member -- the D2 ship-target list \
                 and the workspace have diverged, which is a census defect, not a pass"
            )
        });
        for (fname, entries) in &member.features {
            if fname == "reflect" {
                violations.push(format!("{ship}: declares a `reflect` feature"));
            }
            for entry in entries {
                if cross_entry(entry).is_some_and(|(_, _, f)| f == "reflect") {
                    violations.push(format!(
                        "{ship}: feature `{fname}` forwards `{entry}`"
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "C5 (ship targets are clean) is violated -- a `reflect` feature on a ship target is \
         one `--features` away from shipping the editor's reflection layer. {B12_RULE}:\n  {}",
        violations.join("\n  ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// C6 — every enabling invocation is a named row.
// ─────────────────────────────────────────────────────────────────────────────────────

/// **C6**: every command line under `.github/`, `scripts/` and
/// `docs/REFLECTION-PLAN-GATES.md` that enables a `reflect` feature appears in
/// `tests/reflect_ci_coverage.rs`'s named list. This is F17's counter-clause: `hwrt` has
/// feature-gated bodies compiled by NO CI leg (`grep -c hwrt ci.yml` = 0, measured), and
/// this clause exists so `reflect` cannot inherit that defect. The scanner and the
/// named-list reader live in `tests/reflect_scan_support/mod.rs`, shared with the
/// coverage test's G4 half so the clause's two directions run over the same scan.
#[test]
fn c6_every_enabling_invocation_is_named() {
    let found = scan_scope();
    let named = named_specs();
    let unnamed: Vec<String> = found
        .iter()
        .filter(|e| !named.iter().any(|n| n == &e.spec))
        .map(|e| format!("{}:{} enables `{}`", e.file, e.line, e.spec))
        .collect();

    assert!(
        unnamed.is_empty(),
        "C6 (every enabling invocation is a leg) is violated -- these command lines enable \
         a `reflect` feature but appear in no named row of tests/reflect_ci_coverage.rs, \
         which is exactly how `hwrt` ended up with feature-gated bodies compiled by \
         nothing:\n  {}\n(named rows today: {:?})",
        unnamed.join("\n  "),
        named
    );

    // Non-vacuity for the scan itself: the plan document guarantees at least one
    // enabling invocation exists, so a scan that finds zero has lost its subject.
    assert!(
        !found.is_empty(),
        "C6's scan found NO enabling command line anywhere in scope -- the subject \
         vanished while the check stayed runnable, which reads exactly like a pass and is \
         not one"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// Non-vacuity (mandatory, not optional).
// ─────────────────────────────────────────────────────────────────────────────────────

/// The census's own present-control: each clause above must have a live subject.
/// *"A check whose subject can vanish while the check stays green is not a check"*
/// (`tests/trybuild_corpus_compiler_witness.rs`, the corpus-non-emptiness rule).
#[test]
fn census_is_not_vacuous() {
    let members = members();

    let declaring: Vec<&str> = members
        .values()
        .filter(|m| m.feature("reflect").is_some())
        .map(|m| m.name.as_str())
        .collect();
    assert!(
        !declaring.is_empty(),
        "no workspace member declares a `reflect` feature -- C1/C3/C5 have no subject and \
         their green certifies nothing"
    );

    let optional_edges = members
        .values()
        .flat_map(|m| m.deps.iter())
        .filter(|d| d.name == REFLECT_CRATE && d.optional)
        .count();
    assert!(
        optional_edges > 0,
        "no member carries an optional `{REFLECT_CRATE}` edge -- C2/C4 have no subject"
    );

    let umbrella = members.values().any(|m| {
        m.features.iter().any(|(_, entries)| {
            entries.iter().any(|e| cross_entry(e).is_some_and(|(_, _, f)| f == "reflect"))
        })
    });
    assert!(
        umbrella,
        "no member forwards `<pkg>/reflect` -- the leaf umbrella (B.13 #1, \
         `reflect-dogfood`) is gone and the dogfood enablement path with it"
    );
}
