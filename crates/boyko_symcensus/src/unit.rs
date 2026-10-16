//! V11 G-UNIT (IB design D14 as patched by Rev 5, Rev 6 and ruling 15): the closed normalisation
//! list of (d2)/(d3), (f)'s diagnostic match, the `ug15-ib-receipt` and control-receipt formats, the
//! legs (a), (b), (c), (e), (f) of `ug15 unit`, the certification rule, and `ug15 unit-compare`.
//!
//! # The normalisation list is closed (Rev 6 (d), O13, O17)
//!
//! * (d2), a workspace closure crate's `Running` line: exactly two substitutions, each in both path
//!   separator forms — the tree's `CARGO_TARGET_DIR` → `<T>` and its root → `<R>`, the LONGER string
//!   first (O17: when the target dir lies under the root, root-first would leave `<R>/target` on one
//!   side and `<T>` on the other). Nothing is removed: `-C metadata`, `-C extra-filename` and every
//!   `--extern` file name are tree-independent (cargo hashes a path package's id relative to the
//!   workspace root), so they must match.
//!
//!   Its premise is checked, not assumed (orchestrator ruling 61): each of the two paths is written
//!   in ONE separator form. A mixed form (`D:/s/rc\<id>\target`, which `Path::join` makes of a
//!   forward-slash base on Windows) equals neither substituted form, so it would read as a
//!   difference on every line. [`UnitRun::start`] is therefore RED [`Usage`](RedKind::Usage) on a
//!   mixed `Ctx.target` or `Ctx.root`, and [`unit_compare`] refuses a receipt that carries one
//!   ([`check_single_form`]). The list itself is unchanged.
//! * (d3), a final unit's line: (d2), then exactly these: `-C metadata=…` and `-C extra-filename=…`
//!   dropped (they hash the enabled pin feature); `ib-pin-*` dropped from `--cfg feature=` and from the
//!   `--check-cfg` values; the one `-C link-arg=/MAP:…` value → `<MAP>` (the retain directory, per
//!   tree and run by design). Each side must carry exactly one `/MAP:` token, or RED.
//!
//! Nothing else is normalised. A difference the list does not name stays a difference.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::closure::{self, Graph, Node};
use crate::host;
use crate::json::{self, Json};
use crate::key::{self, Key};
use crate::objbuild::{self, ArtifactKind, Ctx, Request, Subject};
use crate::plain::{self, Verdict};
use crate::red::{Red, RedKind, Result};
use crate::retain::{self, LibRow, RetainDir, Retained};
use crate::sha256;

/// The receipt format line, compared by (d4).
pub const RECEIPT_FORMAT: &str = "ug15-ib-receipt v1";

/// The diagnostic (f) requires on every runner build: rustc's `unused_crate_dependencies` for the
/// host, which fires exactly when no path names the extern, i.e. when rustc never loads it.
pub const F_NEEDLE: &str = "extern crate `boyko_app` is unused in crate `jolt_parity_pyramid`";

/// Whether `path` carries both separator forms, `/` and `\`.
#[must_use]
pub fn mixes_separators(path: &str) -> bool {
    path.contains('/') && path.contains('\\')
}

/// (d2)'s premise (orchestrator ruling 61): `path`, the tree's `CARGO_TARGET_DIR` or its root, is in
/// one separator form. RED [`Usage`](RedKind::Usage) when it mixes `/` and `\`, because neither of
/// the two forms [`normalize_d2`] substitutes would match it.
pub fn check_single_form(what: &str, path: &str) -> Result<()> {
    if mixes_separators(path) {
        return Err(Red::new(
            RedKind::Usage,
            format!(
                "{what} `{path}` mixes `/` and `\\`: (d2) substitutes a path in its two single-separator forms only, so a mixed form would read as a difference on every line. Write it in one form"
            ),
        ));
    }
    Ok(())
}

/// (d2): `target` → `<T>` and `root` → `<R>`, longer string first, both separator forms.
#[must_use]
pub fn normalize_d2(line: &str, target: &str, root: &str) -> String {
    let mut subs = [(target, "<T>"), (root, "<R>")];
    // Stable: on equal lengths the target dir stays first.
    subs.sort_by_key(|s| std::cmp::Reverse(s.0.len()));
    let mut s = line.to_owned();
    for (from, to) in subs {
        if from.is_empty() {
            continue;
        }
        for form in [from.replace('\\', "/"), from.replace('/', "\\")] {
            s = s.replace(&form, to);
        }
    }
    s
}

/// A cargo `Running` line's words, split on spaces outside single quotes; quotes are kept.
#[must_use]
pub fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                cur.push(c);
            }
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(w: &str) -> &str {
    w.strip_prefix('\'').and_then(|w| w.strip_suffix('\'')).unwrap_or(w)
}

fn strip_pins_from_check_cfg(v: &str) -> String {
    // `cfg(feature, values("a", "ib-pin-x", "b"))` → `cfg(feature, values("a", "b"))`.
    let Some(open) = v.find("values(") else { return v.to_owned() };
    let Some(close) = v[open..].find(')').map(|i| open + i) else { return v.to_owned() };
    let inner = &v[open + 7..close];
    let kept: Vec<&str> = inner.split(", ").filter(|item| !item.trim_matches('"').starts_with("ib-pin-")).collect();
    format!("{}{}{}", &v[..open + 7], kept.join(", "), &v[close..])
}

/// `w`, re-quoted as `like` was: (d3) changes a word's content only, never cargo's quoting of it.
fn requote(like: &str, w: &str) -> String {
    if like.starts_with('\'') { format!("'{w}'") } else { w.to_owned() }
}

/// (d3): (d2), then the closed list of final-line changes. RED unless the line carries exactly one
/// `-C link-arg=/MAP:…` token.
pub fn normalize_d3(line: &str, target: &str, root: &str) -> Result<String> {
    let ws = words(&normalize_d2(line, target, root));
    let mut out: Vec<String> = Vec::with_capacity(ws.len());
    let mut maps = 0usize;
    let mut i = 0usize;
    while i < ws.len() {
        let w = unquote(&ws[i]);
        let next = ws.get(i + 1).map(|n| (n.as_str(), unquote(n)));
        if w == "-C" && next.is_some_and(|(_, n)| n.starts_with("metadata=") || n.starts_with("extra-filename=")) {
            i += 2;
            continue;
        }
        if w == "--cfg" && next.is_some_and(|(_, n)| n.starts_with("feature=\"ib-pin-")) {
            i += 2;
            continue;
        }
        if w == "--check-cfg"
            && let Some((raw, n)) = next
        {
            out.push(ws[i].clone());
            out.push(requote(raw, &strip_pins_from_check_cfg(n)));
            i += 2;
            continue;
        }
        if w == "-C"
            && let Some((raw, n)) = next
            && n.starts_with("link-arg=/MAP:")
        {
            maps += 1;
            out.push(ws[i].clone());
            out.push(requote(raw, "link-arg=/MAP:<MAP>"));
            i += 2;
            continue;
        }
        if w.contains("/MAP:") {
            maps += 1;
        }
        out.push(ws[i].clone());
        i += 1;
    }
    if maps != 1 {
        return Err(Red::new(
            RedKind::Mismatch,
            format!("(d3) a final line must carry exactly one `-C link-arg=/MAP:` token; this one carries {maps}"),
        ));
    }
    Ok(out.join(" "))
}

/// The words in `a` that are not in `b` (multiset difference), for a per-line verdict.
#[must_use]
pub fn word_difference(a: &str, b: &str) -> Vec<String> {
    let mut rest: Vec<&str> = b.split(' ').collect();
    let mut out = Vec::new();
    for w in a.split(' ') {
        if let Some(i) = rest.iter().position(|r| *r == w) {
            rest.swap_remove(i);
        } else {
            out.push(w.to_owned());
        }
    }
    out
}

/// Removes ANSI escape sequences (`ESC [ … <final byte>`).
#[must_use]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// (f): the diagnostic line naming the host unused, if the final unit's stderr holds one. Green
/// needs this positive match, so an `allow`, a renamed lint or a lost line all read as RED.
#[must_use]
pub fn host_unused_line(stderr_lines: &[String]) -> Option<String> {
    stderr_lines.iter().map(|l| strip_ansi(l)).find(|l| l.contains(F_NEEDLE))
}

/// One closure row of a receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureRow {
    /// Crate name.
    pub crate_name: String,
    /// cargo's package id.
    pub package_id: String,
    /// `lib` or `proc-macro`.
    pub kind: String,
    /// A workspace member.
    pub workspace: bool,
}

/// One library unit of one retained build, as a receipt records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibLine {
    /// The build's tag.
    pub tag: String,
    /// Crate name.
    pub crate_name: String,
    /// cargo's package id.
    pub package_id: String,
    /// `lib` or `proc-macro`.
    pub kind: String,
    /// Built for the host.
    pub host: bool,
    /// The file.
    pub path: String,
    /// Its sha256.
    pub sha256: String,
}

impl LibLine {
    /// The file name (`-C extra-filename` carried in it): what (d1) compares across trees.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['\\', '/']).next().unwrap_or(&self.path)
    }
}

/// A `ug15 unit` / `unit-arm` receipt: what V11 read, in a form `unit-compare` reads back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    /// The command that wrote it.
    pub command: String,
    /// The instrument key.
    pub key: String,
    /// The key's components, as printed.
    pub key_lines: Vec<String>,
    /// `Ctx.target`.
    pub target: String,
    /// `Ctx.root`.
    pub root: String,
    /// `HEAD <sha>, <n> uncommitted path(s)`.
    pub tree: String,
    /// `BOYKO_PROFILE`, or `unset`.
    pub boyko_profile: String,
    /// `rustc -vV`, one line each.
    pub rustc_vv: Vec<String>,
    /// `cargo -V`.
    pub cargo_v: String,
    /// sha256 of the `\r\n`-normalised `.cargo/config.toml`.
    pub config_sha256: String,
    /// sha256 of the `\r\n`-normalised `Cargo.lock`.
    pub lock_sha256: String,
    /// boyko-physics's resolved normal closure.
    pub closure: Vec<ClosureRow>,
    /// `(tag, subject key, features)` of every build, in build order.
    pub builds: Vec<(String, String, String)>,
    /// Every library unit of every build.
    pub libs: Vec<LibLine>,
    /// `(tag, crate, line)`: every `Running` line of every build.
    pub running: Vec<(String, String, String)>,
    /// `(tag, line)`: each build's final unit line.
    pub finals: Vec<(String, String)>,
    /// `(tag, line)`: each build's stderr diagnostics, ANSI stripped.
    pub diags: Vec<(String, String)>,
    /// `(tag, what, sha256)`: the retained set.
    pub retained: Vec<(String, String, String)>,
    /// `(leg, green, detail)`.
    pub legs: Vec<(String, bool, String)>,
    /// Where the instrument's certification came from, or `none`.
    pub certified: String,
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

impl Receipt {
    /// The text form.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = format!("{RECEIPT_FORMAT}\n");
        let mut row = |fields: &[&str]| {
            let v: Vec<String> = fields.iter().map(|f| clean(f)).collect();
            s.push_str(&v.join("\t"));
            s.push('\n');
        };
        row(&["command", &self.command]);
        row(&["key", &self.key]);
        for l in &self.key_lines {
            row(&["keyline", l]);
        }
        row(&["target", &self.target]);
        row(&["root", &self.root]);
        row(&["tree", &self.tree]);
        row(&["boyko_profile", &self.boyko_profile]);
        for l in &self.rustc_vv {
            row(&["rustc-vv", l]);
        }
        row(&["cargo-v", &self.cargo_v]);
        row(&["config-sha256", &self.config_sha256]);
        row(&["lock-sha256", &self.lock_sha256]);
        for c in &self.closure {
            row(&["closure", &c.crate_name, &c.package_id, &c.kind, if c.workspace { "workspace" } else { "third-party" }]);
        }
        for (t, k, f) in &self.builds {
            row(&["build", t, k, f]);
        }
        for l in &self.libs {
            row(&["lib", &l.tag, &l.crate_name, &l.package_id, &l.kind, if l.host { "host" } else { "target" }, &l.path, &l.sha256]);
        }
        for (t, c, l) in &self.running {
            row(&["running", t, c, l]);
        }
        for (t, l) in &self.finals {
            row(&["final", t, l]);
        }
        for (t, l) in &self.diags {
            row(&["diag", t, l]);
        }
        for (t, w, h) in &self.retained {
            row(&["retained", t, w, h]);
        }
        for (id, g, d) in &self.legs {
            row(&["leg", id, if *g { "GREEN" } else { "RED" }, d]);
        }
        row(&["certified", &self.certified]);
        s
    }

    /// Reads a receipt written by [`Receipt::to_text`]; RED on any other format.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("").trim_end_matches('\r');
        if first != RECEIPT_FORMAT {
            return Err(Red::new(RedKind::Malformed, format!("not a `{RECEIPT_FORMAT}` receipt (first line `{first}`)")));
        }
        let mut r = Self::default();
        for (n, line) in lines.enumerate() {
            let line = line.trim_end_matches('\r');
            // `#` lines are the human summary `ug15 unit` appends after the receipt proper.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let bad = || Red::new(RedKind::Malformed, format!("receipt line {}: `{line}`", n + 2));
            let one = |i: usize| f.get(i).map(|s| (*s).to_owned()).ok_or_else(bad);
            match f[0] {
                "command" => r.command = one(1)?,
                "key" => r.key = one(1)?,
                "keyline" => r.key_lines.push(one(1)?),
                "target" => r.target = one(1)?,
                "root" => r.root = one(1)?,
                "tree" => r.tree = one(1)?,
                "boyko_profile" => r.boyko_profile = one(1)?,
                "rustc-vv" => r.rustc_vv.push(one(1)?),
                "cargo-v" => r.cargo_v = one(1)?,
                "config-sha256" => r.config_sha256 = one(1)?,
                "lock-sha256" => r.lock_sha256 = one(1)?,
                "closure" => r.closure.push(ClosureRow {
                    crate_name: one(1)?,
                    package_id: one(2)?,
                    kind: one(3)?,
                    workspace: match one(4)?.as_str() {
                        "workspace" => true,
                        "third-party" => false,
                        _ => return Err(bad()),
                    },
                }),
                "build" => r.builds.push((one(1)?, one(2)?, one(3)?)),
                "lib" => r.libs.push(LibLine {
                    tag: one(1)?,
                    crate_name: one(2)?,
                    package_id: one(3)?,
                    kind: one(4)?,
                    host: match one(5)?.as_str() {
                        "host" => true,
                        "target" => false,
                        _ => return Err(bad()),
                    },
                    path: one(6)?,
                    sha256: one(7)?,
                }),
                "running" => r.running.push((one(1)?, one(2)?, one(3)?)),
                "final" => r.finals.push((one(1)?, one(2)?)),
                "diag" => r.diags.push((one(1)?, one(2)?)),
                "retained" => r.retained.push((one(1)?, one(2)?, one(3)?)),
                "leg" => r.legs.push((
                    one(1)?,
                    match one(2)?.as_str() {
                        "GREEN" => true,
                        "RED" => false,
                        _ => return Err(bad()),
                    },
                    one(3)?,
                )),
                "certified" => r.certified = one(1)?,
                _ => return Err(bad()),
            }
        }
        Ok(r)
    }

    /// The library units of the build tagged `tag`.
    #[must_use]
    pub fn libs_of(&self, tag: &str) -> Vec<&LibLine> {
        self.libs.iter().filter(|l| l.tag == tag).collect()
    }

    /// The final line of the build tagged `tag`.
    #[must_use]
    pub fn final_of(&self, tag: &str) -> Option<&str> {
        self.finals.iter().find(|(t, _)| t == tag).map(|(_, l)| l.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// `ug15 unit` (V11 on one tree) and `ug15 unit-compare` (V11 (d) across two trees)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The controls a receipt must record as expected to certify an instrument (IB P0r: RC-11 (a)-(f),
/// RC-13, RC-14 (a)-(c), green controls (b) and (d); orchestrator ruling 55 Q3).
pub const REQUIRED_CONTROLS: [&str; 12] =
    ["g-b", "rc11c", "rc13", "rc14a", "rc14b", "rc11a", "rc11b", "rc11d", "rc11e", "rc11f", "rc14c", "g-d"];

/// The control receipt's format line.
pub const CONTROLS_FORMAT: &str = "ug15-ib-controls v1";

/// The committed control receipt for `k8`, relative to the workspace root.
#[must_use]
pub fn committed_controls_path(k8: &str) -> PathBuf {
    PathBuf::from(format!("crates/boyko_symcensus/ug15/receipts/ib/controls-{k8}.txt"))
}

/// One control's line in a control receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlLine {
    /// `g-b`, `rc11a`, …
    pub id: String,
    /// The observed verdict was the expected one.
    pub as_expected: bool,
    /// The expected verdict, in one line.
    pub expected: String,
    /// The observed verdict, in one line.
    pub observed: String,
}

/// A `ug15 unit-controls` receipt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlsReceipt {
    /// The instrument key the controls ran under.
    pub key: String,
    /// The key's components.
    pub key_lines: Vec<String>,
    /// The tree line.
    pub tree: String,
    /// Written by `--only`: never certifies.
    pub partial: bool,
    /// Every control run.
    pub controls: Vec<ControlLine>,
    /// `(id, line)`: what each control measured.
    pub details: Vec<(String, String)>,
    /// `(id, line)`: each control's edit.
    pub edits: Vec<(String, String)>,
}

impl ControlsReceipt {
    /// The text form.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = format!("{CONTROLS_FORMAT}\n");
        let mut row = |f: &[&str]| {
            let v: Vec<String> = f.iter().map(|x| clean(x)).collect();
            s.push_str(&v.join("\t"));
            s.push('\n');
        };
        row(&["key", &self.key]);
        for l in &self.key_lines {
            row(&["keyline", l]);
        }
        row(&["tree", &self.tree]);
        row(&["partial", if self.partial { "yes" } else { "no" }]);
        for c in &self.controls {
            row(&["control", &c.id, if c.as_expected { "as-expected" } else { "UNEXPECTED" }, &c.expected, &c.observed]);
        }
        for (id, l) in &self.edits {
            row(&["edit", id, l]);
        }
        for (id, l) in &self.details {
            row(&["detail", id, l]);
        }
        let unexpected = self.controls.iter().filter(|c| !c.as_expected).count();
        let result = if unexpected == 0 { "all as expected".to_owned() } else { format!("{unexpected} unexpected") };
        row(&["result", &result]);
        s
    }

    /// Reads a control receipt; RED on any other format.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("").trim_end_matches('\r');
        if first != CONTROLS_FORMAT {
            return Err(Red::new(RedKind::Malformed, format!("not a `{CONTROLS_FORMAT}` receipt (first line `{first}`)")));
        }
        let mut r = Self::default();
        let mut result = None;
        for line in lines.map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
            let f: Vec<&str> = line.split('\t').collect();
            let bad = || Red::new(RedKind::Malformed, format!("control receipt line `{line}`"));
            let one = |i: usize| f.get(i).map(|s| (*s).to_owned()).ok_or_else(bad);
            match f[0] {
                "key" => r.key = one(1)?,
                "keyline" => r.key_lines.push(one(1)?),
                "tree" => r.tree = one(1)?,
                "partial" => {
                    r.partial = match one(1)?.as_str() {
                        "no" => false,
                        "yes" => true,
                        _ => return Err(bad()),
                    }
                }
                "control" => r.controls.push(ControlLine {
                    id: one(1)?,
                    as_expected: match one(2)?.as_str() {
                        "as-expected" => true,
                        "UNEXPECTED" => false,
                        _ => return Err(bad()),
                    },
                    expected: one(3)?,
                    observed: one(4)?,
                }),
                "edit" => r.edits.push((one(1)?, one(2)?)),
                "detail" => r.details.push((one(1)?, one(2)?)),
                "result" => result = Some(one(1)?),
                _ => return Err(bad()),
            }
        }
        let unexpected = r.controls.iter().filter(|c| !c.as_expected).count();
        let want = if unexpected == 0 { "all as expected".to_owned() } else { format!("{unexpected} unexpected") };
        if result.as_deref() != Some(want.as_str()) {
            return Err(Red::new(RedKind::Malformed, format!("the control receipt's result line is {result:?}, its controls say `{want}`")));
        }
        Ok(r)
    }

    /// `Ok(())` when this receipt certifies `key`: complete, the same key, every required control
    /// present and as expected. Otherwise the reason it does not.
    pub fn certifies(&self, key: &str) -> std::result::Result<(), String> {
        if self.partial {
            return Err("it is partial (`--only`); a partial receipt never certifies".to_owned());
        }
        if self.key != key {
            return Err(format!("it certifies key {}, this instrument is {key}", self.key));
        }
        for id in REQUIRED_CONTROLS {
            match self.controls.iter().find(|c| c.id == id) {
                None => return Err(format!("control {id} is not in it")),
                Some(c) if !c.as_expected => return Err(format!("control {id} was UNEXPECTED: {}", c.observed)),
                Some(_) => {}
            }
        }
        Ok(())
    }
}

/// Where this instrument's certification comes from: the committed receipt `controls-<K8>.txt`,
/// tracked and clean, or `--controls <path>` outside both trees (the window directory, W13).
/// `Ok(source)` or `Err(why not)`.
pub fn certification(root: &Path, target: &Path, key: &Key, controls: Option<&Path>) -> std::result::Result<String, String> {
    let (path, from) = match controls {
        Some(p) => {
            if !p.is_absolute() || crate::retain::is_within(p, root) || crate::retain::is_within(p, target) {
                return Err(format!(
                    "--controls {} must be absolute and outside the tree and the target dir (the window directory); the committed receipt is the only in-tree location",
                    p.display()
                ));
            }
            (p.to_path_buf(), format!("--controls {}", p.display()))
        }
        None => {
            let rel = committed_controls_path(key.k8());
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            let tracked = Command::new("git").current_dir(root).args(["ls-files", "--error-unmatch", &rel_s]).output();
            if !tracked.is_ok_and(|o| o.status.success()) {
                return Err(format!("{rel_s} is not a tracked file of this tree"));
            }
            let st = Command::new("git").current_dir(root).args(["status", "--porcelain", "--", &rel_s]).output();
            match st {
                Ok(o) if o.status.success() && o.stdout.is_empty() => {}
                _ => return Err(format!("{rel_s} has uncommitted changes")),
            }
            (root.join(&rel), rel_s)
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = ControlsReceipt::parse(&text).map_err(|e| e.to_string())?;
    r.certifies(&key.hex)?;
    Ok(from)
}

/// sha256 of a file's `\r\n`-normalised bytes (V11 (d4): a raw hash would measure the checkout).
pub fn normalized_sha256(path: &Path) -> Result<String> {
    let b = std::fs::read(path).map_err(|e| Red::io(path, &e))?;
    Ok(sha256::bytes_hex(&key::normalize_eol(&b)))
}

/// The `ENGINE_PACKAGES` list, read from `crates/boyko_diag/src/sample.rs` with `syn` (this crate
/// links no engine crate; V11 (a) needs the list the probe's optional set is checked against).
pub fn engine_packages(root: &Path) -> Result<Vec<String>> {
    let p = root.join("crates/boyko_diag/src/sample.rs");
    let text = std::fs::read_to_string(&p).map_err(|e| Red::io(&p, &e))?;
    let file = syn::parse_file(&text).map_err(|e| Red::new(RedKind::Malformed, format!("{}: {e}", p.display())))?;
    for item in &file.items {
        if let syn::Item::Const(c) = item
            && c.ident == "ENGINE_PACKAGES"
        {
            let syn::Expr::Reference(r) = &*c.expr else { break };
            let syn::Expr::Array(a) = &*r.expr else { break };
            let mut out = Vec::new();
            for e in &a.elems {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = e {
                    out.push(s.value());
                } else {
                    return Err(Red::new(RedKind::Malformed, "ENGINE_PACKAGES holds a non-literal element"));
                }
            }
            if out.is_empty() {
                return Err(Red::new(RedKind::EmptyCensus, "ENGINE_PACKAGES is empty"));
            }
            return Ok(out);
        }
    }
    Err(Red::new(RedKind::Malformed, format!("{} has no `const ENGINE_PACKAGES: &[&str] = &[…]`", p.display())))
}

fn dep_str(d: &Json, k: &str) -> Option<String> {
    d.get(k).and_then(Json::as_str).map(str::to_owned)
}

/// V11 (a), the manifest half: both packages have the game shape (IB design Rev 5 (a), Rev 6 (a)).
pub fn leg_a_manifests(ctx: &Ctx, graph: &Graph) -> Result<Verdict> {
    let out = Command::new(crate::objbuild::cargo_program())
        .current_dir(&ctx.root)
        .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .env("CARGO_TARGET_DIR", &ctx.target)
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo metadata: {e}")))?;
    if !out.status.success() {
        return Err(Red::new(RedKind::BuildFailed, format!("cargo metadata --no-deps failed: {}", String::from_utf8_lossy(&out.stderr))));
    }
    let meta = json::parse(&String::from_utf8_lossy(&out.stdout))?;
    let closure_names: BTreeSet<String> = graph.closure()?.into_iter().map(|n| n.name).collect();
    let engine = engine_packages(&ctx.root)?;
    let want_optional: BTreeSet<String> =
        engine.iter().filter(|p| !["boyko-reflect", "boyko-app", "boyko-physics", "boyko-ecs"].contains(&p.as_str())).cloned().collect();
    let mut bad: Vec<String> = Vec::new();
    for (pkg_name, is_runner) in [("boyko-physics-parity", true), ("boyko-physics-probe", false)] {
        let Some(pkg) = meta.get("packages").map(Json::items).unwrap_or(&[]).iter().find(|p| p.get("name").and_then(Json::as_str) == Some(pkg_name)) else {
            bad.push(format!("(a) no package {pkg_name}"));
            continue;
        };
        let (mut plain, mut windows, mut optional) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for d in pkg.get("dependencies").map(Json::items).unwrap_or(&[]) {
            let name = dep_str(d, "name").unwrap_or_default();
            let features: Vec<&str> = d.get("features").map(Json::items).unwrap_or(&[]).iter().filter_map(Json::as_str).collect();
            let opt = d.get("optional").and_then(Json::as_bool) == Some(true);
            let target = dep_str(d, "target");
            if !matches!(d.get("kind"), None | Some(Json::Null)) {
                bad.push(format!("(a) {pkg_name}: `{name}` is not a normal dependency (no [dev-dependencies] or [build-dependencies] in the game shape)"));
            }
            if !matches!(d.get("rename"), None | Some(Json::Null)) {
                bad.push(format!("(a) {pkg_name}: `{name}` is renamed"));
            }
            if !features.is_empty() || d.get("uses_default_features").and_then(Json::as_bool) != Some(true) {
                bad.push(format!("(a) {pkg_name}: `{name}` carries a feature key ({features:?}, default-features {:?})", d.get("uses_default_features")));
            }
            let registry = dep_str(d, "source").is_some_and(|s| s.starts_with("registry+"));
            if is_runner && name == "mimalloc" {
                if !registry || dep_str(d, "req").as_deref() != Some("^0.1") || !opt || target.is_some() {
                    bad.push(format!("(a) {pkg_name}: mimalloc is not the moved entry (crates.io, ^0.1, optional, no target): {d:?}"));
                }
            } else if !matches!(d.get("source"), None | Some(Json::Null)) {
                bad.push(format!("(a) {pkg_name}: `{name}` is not a path dependency"));
            }
            match (target.as_deref(), opt) {
                (Some("cfg(windows)"), false) => {
                    windows.insert(name);
                }
                (None, false) => {
                    plain.insert(name);
                }
                (None, true) => {
                    optional.insert(name);
                }
                (t, o) => bad.push(format!("(a) {pkg_name}: `{name}` has target {t:?}, optional {o}")),
            }
        }
        let features = pkg.get("features");
        let feature = |k: &str| -> Option<Vec<String>> { features.and_then(|f| f.get(k)).map(|v| v.items().iter().filter_map(Json::as_str).map(str::to_owned).collect()) };
        let feature_keys: BTreeSet<String> = match features {
            Some(Json::Object(kv)) => kv.iter().map(|(k, _)| k.clone()).collect(),
            _ => BTreeSet::new(),
        };
        if windows != BTreeSet::from(["boyko-app".to_owned()]) {
            bad.push(format!("(a) {pkg_name}: the cfg(windows) entries are {windows:?}, not exactly boyko-app"));
        }
        if feature("default") != Some(Vec::new()) {
            bad.push(format!("(a) {pkg_name}: `default` is {:?}, not []", feature("default")));
        }
        if is_runner {
            if let Some(stray) = plain.iter().find(|n| !closure_names.contains(*n)) {
                bad.push(format!("(a) {pkg_name}: `{stray}` is not in boyko-physics's resolved normal closure"));
            }
            if optional != BTreeSet::from(["mimalloc".to_owned()]) {
                bad.push(format!("(a) {pkg_name}: the optional entries are {optional:?}, not exactly mimalloc"));
            }
            let want_keys: BTreeSet<String> = ["default", "bench-alloc", "narrowphase-counts"].iter().map(|s| (*s).to_owned()).collect();
            if feature_keys != want_keys
                || feature("bench-alloc") != Some(vec!["dep:mimalloc".to_owned()])
                || feature("narrowphase-counts") != Some(vec!["boyko-physics/narrowphase-counts".to_owned()])
            {
                bad.push(format!("(a) {pkg_name}: the features are not exactly default = [], bench-alloc = [dep:mimalloc], narrowphase-counts = [boyko-physics/narrowphase-counts]: {features:?}"));
            }
        } else {
            if plain != BTreeSet::from(["boyko-physics".to_owned(), "boyko-ecs".to_owned()]) {
                bad.push(format!("(a) {pkg_name}: the non-optional entries are {plain:?}, not exactly boyko-physics and boyko-ecs"));
            }
            if optional != want_optional {
                bad.push(format!(
                    "(a) {pkg_name}: the optional entries differ from ENGINE_PACKAGES minus boyko-reflect, boyko-app, boyko-physics, boyko-ecs: extra {:?}, missing {:?}",
                    optional.difference(&want_optional).collect::<Vec<_>>(),
                    want_optional.difference(&optional).collect::<Vec<_>>()
                ));
            }
            let mut wide = feature("scope-wide").unwrap_or_default();
            wide.sort();
            let mut want_wide: Vec<String> = want_optional.iter().map(|n| format!("dep:{n}")).collect();
            want_wide.sort();
            let want_keys: BTreeSet<String> = ["default", "scope-wide"].iter().map(|s| (*s).to_owned()).collect();
            if feature_keys != want_keys || wide != want_wide {
                bad.push(format!("(a) {pkg_name}: the features are not exactly default = [] and scope-wide = the optional set through dep: ({features:?})"));
            }
        }
    }
    Ok(if bad.is_empty() {
        Verdict::green(format!(
            "(a) GREEN: both manifests have the game shape; the runner's one registry entry is mimalloc ^0.1 behind bench-alloc; the probe's optional set is the {} ENGINE_PACKAGES crates scope-wide activates",
            want_optional.len()
        ))
    } else {
        Verdict::red(bad)
    })
}

/// V11 (a), the build half: no timed or V9 subject carries a diagnostic feature in its final line.
#[must_use]
pub fn leg_a_final_lines(finals: &[(&str, &str)]) -> Verdict {
    let mut bad = Vec::new();
    for (tag, line) in finals {
        for f in ["scope-wide", "bench-alloc", "narrowphase-counts"] {
            if line.contains(&format!("feature=\"{f}\"")) || line.contains(&format!("feature=\\\"{f}\\\"")) {
                bad.push(format!("(a) the {tag} build's final line enables `{f}`"));
            }
        }
    }
    if bad.is_empty() { Verdict::green("(a) GREEN: no diagnostic feature in the runner's or the probe's final line".to_owned()) } else { Verdict::red(bad) }
}

/// One library unit's identity across two builds of one tree.
fn unit_key(l: &LibRow) -> (String, &'static str, bool) {
    (l.package_id.clone(), l.kind.label(), l.host)
}

/// The per-unit comparison of two library tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableDiff {
    /// Units in both with the same path and sha256, by package id.
    pub same: Vec<String>,
    /// Units whose path differs (`-C metadata` moved): `(package id, crate, a path, b path)`.
    pub path_differs: Vec<(String, String, String, String)>,
    /// Units at one path whose bytes differ: `(package id, crate)`.
    pub sha_differs: Vec<(String, String)>,
    /// Units only in `a` / only in `b`: `(package id, crate)`.
    pub only_a: Vec<(String, String)>,
    /// See [`TableDiff::only_a`].
    pub only_b: Vec<(String, String)>,
}

impl TableDiff {
    /// No difference.
    #[must_use]
    pub fn is_equal(&self) -> bool {
        self.path_differs.is_empty() && self.sha_differs.is_empty() && self.only_a.is_empty() && self.only_b.is_empty()
    }

    /// Every package id with any difference.
    #[must_use]
    pub fn differing_ids(&self) -> BTreeSet<String> {
        let mut s: BTreeSet<String> = self.path_differs.iter().map(|d| d.0.clone()).collect();
        s.extend(self.sha_differs.iter().map(|d| d.0.clone()));
        s.extend(self.only_a.iter().map(|d| d.0.clone()));
        s.extend(self.only_b.iter().map(|d| d.0.clone()));
        s
    }

    /// One line per difference.
    #[must_use]
    pub fn lines(&self, what: &str) -> Vec<String> {
        let mut v = Vec::new();
        for (_, c, a, b) in &self.path_differs {
            v.push(format!("{what} path differs at {c}: {a} vs {b}"));
        }
        for (_, c) in &self.sha_differs {
            v.push(format!("{what} same path, different sha256 at {c}"));
        }
        for (_, c) in &self.only_a {
            v.push(format!("{what} only in the first: {c}"));
        }
        for (_, c) in &self.only_b {
            v.push(format!("{what} only in the second: {c}"));
        }
        v
    }
}

/// Compares two library tables unit by unit (`(package id, kind, host)`), keeping only the units
/// `keep` admits.
pub fn diff_tables(a: &[LibRow], b: &[LibRow], keep: impl Fn(&LibRow) -> bool) -> TableDiff {
    let index = |t: &[LibRow]| -> BTreeMap<(String, &'static str, bool), LibRow> { t.iter().filter(|l| keep(l)).map(|l| (unit_key(l), l.clone())).collect() };
    let (ia, ib) = (index(a), index(b));
    let mut d = TableDiff::default();
    for (k, la) in &ia {
        match ib.get(k) {
            None => d.only_a.push((k.0.clone(), la.crate_name.clone())),
            Some(lb) if la.path != lb.path => {
                d.path_differs.push((k.0.clone(), la.crate_name.clone(), la.path.display().to_string(), lb.path.display().to_string()));
            }
            Some(lb) if la.sha256 != lb.sha256 => d.sha_differs.push((k.0.clone(), la.crate_name.clone())),
            Some(_) => d.same.push(k.0.clone()),
        }
    }
    for (k, lb) in &ib {
        if !ia.contains_key(k) {
            d.only_b.push((k.0.clone(), lb.crate_name.clone()));
        }
    }
    d
}

/// The anti-vacuity floor (Rev 6 W12): the closure crates `table` holds no artifact for (a lib by
/// its rlib, a proc-macro by its dll).
#[must_use]
pub fn floor_missing(table: &[LibRow], closure: &[Node]) -> Vec<String> {
    closure
        .iter()
        .filter(|n| !table.iter().any(|l| l.package_id == n.id && Some(l.kind) == n.kind))
        .map(|n| n.crate_name.clone().unwrap_or_else(|| n.name.clone()))
        .collect()
}

/// V11 (b): the runner's and the probe's tables are equal as (crate, path, sha256), and each
/// covers the closure.
#[must_use]
pub fn leg_b(runner: &Retained, probe: &Retained, closure: &[Node]) -> Verdict {
    let d = diff_tables(&runner.libs, &probe.libs, |_| true);
    let mut bad = d.lines("(b)");
    for (tag, t) in [(&runner.tag, &runner.libs), (&probe.tag, &probe.libs)] {
        let miss = floor_missing(t, closure);
        if !miss.is_empty() {
            bad.push(format!("(b) floor: the {tag} build has no artifact for closure crate(s) {miss:?}"));
        }
    }
    if bad.is_empty() {
        Verdict::green(format!("(b) GREEN: {} library units identical in path and sha256; both cover the {} closure crates", d.same.len(), closure.len()))
    } else {
        Verdict::red(bad)
    }
}

/// V11 (e): probe-wide's table restricted to the closure equals the runner's (sets the label's
/// scope note; never an abort).
#[must_use]
pub fn leg_e(runner: &Retained, wide: &Retained, closure: &[Node]) -> Verdict {
    let ids: BTreeSet<&str> = closure.iter().map(|n| n.id.as_str()).collect();
    let d = diff_tables(&runner.libs, &wide.libs, |l| ids.contains(l.package_id.as_str()));
    let miss = floor_missing(&wide.libs, closure);
    let mut bad = d.lines("(e)");
    if !miss.is_empty() {
        bad.push(format!("(e) floor: probe-wide has no artifact for {miss:?}"));
    }
    if bad.is_empty() {
        Verdict::green(format!(
            "(e) GREEN: probe-wide's closure units equal the runner's ({}). Scope note: a game may also link any ENGINE_PACKAGES crate except boyko-reflect, at default features",
            d.same.len()
        ))
    } else {
        bad.push("(e) scope note narrows to: a game linking only boyko-app and boyko-physics".to_owned());
        Verdict::red(bad)
    }
}

/// V11 (f): the runner's final unit reports the host unused.
#[must_use]
pub fn leg_f(runner: &Retained) -> Verdict {
    match host_unused_line(&runner.built.stderr_lines) {
        Some(l) => Verdict::green(format!("(f) GREEN: {}", l.trim())),
        None => Verdict::red(vec![format!("(f) no `{F_NEEDLE}` among the {} stderr line(s) of the runner build", runner.built.stderr_lines.len())]),
    }
}

/// One IB run on one tree: the key, the closure, the retain directory and the receipt it fills.
pub struct UnitRun<'c> {
    /// The tree and target.
    pub ctx: &'c Ctx,
    /// This instrument's key.
    pub key: Key,
    /// The resolve graph.
    pub graph: Graph,
    /// boyko-physics's closure.
    pub closure: Vec<Node>,
    /// Where builds are retained.
    pub retain: RetainDir,
    /// The receipt.
    pub receipt: Receipt,
}

impl<'c> UnitRun<'c> {
    /// Checks that the target dir and the root are each in one separator form (RED
    /// [`Usage`](RedKind::Usage) otherwise, before anything else: ruling 61), computes the key (RED
    /// [`Stale`](RedKind::Stale) on a stale executable), checks the profile premise, reads the
    /// closure, opens the retain directory and starts the receipt.
    pub fn start(ctx: &'c Ctx, retain: &Path, command: &str) -> Result<Self> {
        check_single_form("the target dir (CARGO_TARGET_DIR)", &ctx.target.to_string_lossy())?;
        check_single_form("the tree root", &ctx.root.to_string_lossy())?;
        let key = key::instrument_key(&ctx.root, &crate::objbuild::cargo_program())?;
        let manifest = ctx.root.join("Cargo.toml");
        retain::check_profile_premise(&std::fs::read_to_string(&manifest).map_err(|e| Red::io(&manifest, &e))?)?;
        let graph = Graph::load(ctx)?;
        let closure = graph.closure()?;
        let retain = RetainDir::open(retain, &ctx.root, &ctx.target)?;
        let receipt = Receipt {
            command: command.to_owned(),
            key: key.hex.clone(),
            key_lines: key.receipt().lines().map(str::to_owned).collect(),
            target: ctx.target.display().to_string(),
            root: ctx.root.display().to_string(),
            tree: host::git_provenance(&ctx.root),
            boyko_profile: std::env::var("BOYKO_PROFILE").unwrap_or_else(|_| "unset".to_owned()),
            rustc_vv: key.rustc_vv.lines().map(str::to_owned).collect(),
            cargo_v: key.cargo_v.clone(),
            config_sha256: normalized_sha256(&ctx.root.join(".cargo").join("config.toml"))?,
            lock_sha256: normalized_sha256(&ctx.root.join("Cargo.lock"))?,
            closure: closure
                .iter()
                .map(|n| ClosureRow {
                    crate_name: n.crate_name.clone().unwrap_or_default(),
                    package_id: n.id.clone(),
                    kind: n.kind.map_or("?", ArtifactKind::label).to_owned(),
                    workspace: n.workspace,
                })
                .collect(),
            certified: "none".to_owned(),
            ..Receipt::default()
        };
        Ok(Self { ctx, key, graph, closure, retain, receipt })
    }

    /// Touches the closure's workspace roots (the next build is V11 (d)'s touched build).
    pub fn touch_closure(&mut self) -> Result<usize> {
        Ok(closure::touch_closure_roots(&self.closure)?.len())
    }

    /// One retained build, recorded in the receipt.
    pub fn build(&mut self, s: Subject, tag: &str, features: &[String], env: &[(String, String)], extra: &[String]) -> Result<Retained> {
        let req = Request::new(s, "release", extra).with_features(features).with_env(env);
        let r = retain::build_retained(self.ctx, &req, &self.retain, tag, &self.key)?;
        self.record(&r, features);
        Ok(r)
    }

    /// Records a retained build in the receipt.
    pub fn record(&mut self, r: &Retained, features: &[String]) {
        let rc = &mut self.receipt;
        rc.builds.push((r.tag.clone(), r.built.subject.key.to_owned(), if features.is_empty() { "-".to_owned() } else { features.join(",") }));
        for l in &r.libs {
            rc.libs.push(LibLine {
                tag: r.tag.clone(),
                crate_name: l.crate_name.clone(),
                package_id: l.package_id.clone(),
                kind: l.kind.label().to_owned(),
                host: l.host,
                path: l.path.display().to_string(),
                sha256: l.sha256.clone(),
            });
        }
        for (c, line) in &r.built.running {
            rc.running.push((r.tag.clone(), c.clone(), line.clone()));
        }
        rc.finals.push((r.tag.clone(), r.built.final_rustc.clone()));
        for l in &r.built.stderr_lines {
            let t = strip_ansi(l);
            if t.contains("warning") || t.contains("error") || t.contains("extern crate") {
                rc.diags.push((r.tag.clone(), t));
            }
        }
        rc.retained.push((r.tag.clone(), "image".to_owned(), r.image_sha256.clone()));
        rc.retained.push((r.tag.clone(), "object".to_owned(), r.object_sha256.clone()));
        rc.retained.push((r.tag.clone(), "map".to_owned(), r.map_sha256.clone()));
        if let Some(p) = &r.pdb_sha256 {
            rc.retained.push((r.tag.clone(), "pdb".to_owned(), p.clone()));
        }
    }

    /// Records a leg's verdict and returns it.
    pub fn leg(&mut self, id: &str, v: Verdict) -> Verdict {
        self.receipt.legs.push((id.to_owned(), v.green, v.lines.join(" | ")));
        v
    }
}

/// `ug15 unit`: V11 (a), (b), (c), (e), (f) on this tree, runner against probe, then the
/// certification. Returns the receipt text and the verdict (`Err` naming the first failing leg,
/// or [`Uncertified`](RedKind::Uncertified)). A red (e) narrows the scope note and is never the
/// verdict (D14: (e) aborts nothing).
pub fn unit(ctx: &Ctx, retain_dir: &Path, controls: Option<&Path>) -> Result<(String, Result<()>)> {
    let mut run = UnitRun::start(ctx, retain_dir, "unit")?;
    let cert = certification(&ctx.root, &ctx.target, &run.key, controls);
    let a = leg_a_manifests(ctx, &run.graph)?;
    let touched = run.touch_closure()?;
    (ctx.echo)(&format!("ug15 unit: touched {touched} workspace closure root(s)"));
    let runner_s = objbuild::subject("parity_runner")?;
    let probe_s = objbuild::subject("physics_probe")?;
    let lint = ["-W".to_owned(), "unused-crate-dependencies".to_owned()];
    let runner = run.build(runner_s, "runner", &[], &[], &lint)?;
    let probe = run.build(probe_s, "probe", &[], &[], &[])?;
    let wide = run.build(probe_s, "probe-wide", &["scope-wide".to_owned()], &[], &[])?;
    let plain = plain::plain_build(ctx, runner_s, &[], &run.retain, &runner)?;
    run.receipt.retained.push((plain.tag.clone(), "image".to_owned(), plain.image_sha256.clone()));
    let finals = [("runner", runner.built.final_rustc.as_str()), ("probe", probe.built.final_rustc.as_str())];
    let a2 = leg_a_final_lines(&finals);
    let a = Verdict { green: a.green && a2.green, lines: a.lines.into_iter().chain(a2.lines).collect() };
    let mut verdicts = vec![("(a)", run.leg("(a)", a))];
    verdicts.push(("(b)", run.leg("(b)", leg_b(&runner, &probe, &run.closure))));
    let c1 = plain::check_c1(&plain, &runner)?;
    let c2 = plain::check_c2(&runner.image, &plain.image)?;
    let c = Verdict { green: c1.green && c2.green, lines: c1.lines.into_iter().chain(c2.lines).collect() };
    verdicts.push(("(c)", run.leg("(c)", c)));
    let e = run.leg("(e)", leg_e(&runner, &wide, &run.closure));
    verdicts.push(("(f)", run.leg("(f)", leg_f(&runner))));
    run.receipt.certified = match &cert {
        Ok(from) => format!("certified: {from}"),
        Err(why) => format!("UNCERTIFIED: {why}"),
    };
    let mut text = run.receipt.to_text();
    text.push_str("\n# legs\n");
    for (id, v) in &verdicts {
        for l in &v.lines {
            text.push_str(&format!("# {id} {l}\n"));
        }
    }
    for l in &e.lines {
        text.push_str(&format!("# (e) {l}\n"));
    }
    text.push_str(&format!("# {}\n", run.receipt.certified));
    let verdict = if let Some((id, v)) = verdicts.iter().find(|(_, v)| !v.green) {
        Err(Red::new(RedKind::Mismatch, format!("V11 {id} RED: {}", v.lines.join(" | "))))
    } else if let Err(why) = &cert {
        Err(Red::new(RedKind::Uncertified, format!("every V11 leg is green, but this instrument (key {}) is uncertified: {why}", run.key.k8())))
    } else {
        Ok(())
    };
    Ok((text, verdict))
}

/// One compared item of `unit-compare`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmp {
    /// What was compared: a crate (with `host`/`target`), a final tag, or an environment item.
    pub item: String,
    /// Host unit (d1 only).
    pub host: bool,
    /// Equal on both trees.
    pub same: bool,
    /// What differs (words, file names or values).
    pub detail: String,
    /// (d2)/(d3): the words in one side's normalised line(s) and not in the other's, pre-only first
    /// (RC-11 (f) reads which `--extern` names moved). Empty for (d1)/(d4) and for an equal item.
    pub words: Vec<String>,
}

/// V11 (d), PRE against TIP.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareReport {
    /// (d1) closure file names, per (crate, host).
    pub d1: Vec<Cmp>,
    /// (d2) workspace closure lines, per crate.
    pub d2: Vec<Cmp>,
    /// (d3) final lines, per tag.
    pub d3: Vec<Cmp>,
    /// (d4) environment and files.
    pub d4: Vec<Cmp>,
    /// Anti-vacuity failures and refusals.
    pub refusals: Vec<String>,
}

impl CompareReport {
    /// Every item equal and nothing refused.
    #[must_use]
    pub fn green(&self) -> bool {
        self.refusals.is_empty() && [&self.d1, &self.d2, &self.d3, &self.d4].iter().all(|v| v.iter().all(|c| c.same))
    }

    /// Whether leg `n` (1..=4) is green.
    #[must_use]
    pub fn leg_green(&self, n: usize) -> bool {
        let v = match n {
            1 => &self.d1,
            2 => &self.d2,
            3 => &self.d3,
            _ => &self.d4,
        };
        v.iter().all(|c| c.same)
    }

    /// The per-item verdict text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (name, v) in [("d1", &self.d1), ("d2", &self.d2), ("d3", &self.d3), ("d4", &self.d4)] {
            let red = v.iter().filter(|c| !c.same).count();
            s.push_str(&format!("({name}) {} item(s), {red} red\n", v.len()));
            for c in v {
                s.push_str(&format!(
                    "  ({name}) {} {}{}{}\n",
                    if c.same { "same" } else { "RED " },
                    c.item,
                    if name == "d1" { if c.host { " [host]" } else { " [target]" } } else { "" },
                    if c.detail.is_empty() { String::new() } else { format!(": {}", c.detail) }
                ));
            }
        }
        for r in &self.refusals {
            s.push_str(&format!("  REFUSED {r}\n"));
        }
        s
    }
}

/// The touched build of a receipt: the one tagged `runner`.
const TOUCHED: &str = "runner";

/// `ug15 unit-compare`: V11 (d1)–(d4) between two receipts, under the closed normalisation list.
pub fn unit_compare(pre: &Receipt, tip: &Receipt) -> CompareReport {
    let mut r = CompareReport::default();
    if !pre.builds.iter().any(|b| b.0 == TOUCHED) || !tip.builds.iter().any(|b| b.0 == TOUCHED) {
        r.refusals.push("a receipt has no `runner` build, the touched build (d1) and (d2) read".to_owned());
        return r;
    }
    // (d2)'s premise (ruling 61): a receipt written before the check, or by another tool, is
    // refused by name instead of reading as a difference on every line.
    for (which, rc) in [("pre", pre), ("tip", tip)] {
        for (what, path) in [("target", &rc.target), ("root", &rc.root)] {
            if let Err(red) = check_single_form(&format!("the {which} receipt's {what}"), path) {
                r.refusals.push(red.to_string());
            }
        }
    }
    let closure_names = |rc: &Receipt| -> BTreeSet<(String, bool)> { rc.closure.iter().map(|c| (c.crate_name.clone(), c.workspace)).collect() };
    if closure_names(pre) != closure_names(tip) {
        r.refusals.push(format!(
            "the two trees' closures differ as crate sets: pre only {:?}, tip only {:?}",
            closure_names(pre).difference(&closure_names(tip)).collect::<Vec<_>>(),
            closure_names(tip).difference(&closure_names(pre)).collect::<Vec<_>>()
        ));
    }
    // (d1): the closure units' file names, per (crate, host), in the touched build.
    let names = |rc: &Receipt| -> BTreeMap<(String, bool), BTreeSet<String>> {
        let ids: BTreeSet<&str> = rc.closure.iter().map(|c| c.package_id.as_str()).collect();
        let mut m: BTreeMap<(String, bool), BTreeSet<String>> = BTreeMap::new();
        for l in rc.libs.iter().filter(|l| l.tag == TOUCHED && ids.contains(l.package_id.as_str())) {
            m.entry((l.crate_name.clone(), l.host)).or_default().insert(l.file_name().to_owned());
        }
        m
    };
    let (np, nt) = (names(pre), names(tip));
    for (which, rc, m) in [("pre", pre, &np), ("tip", tip, &nt)] {
        for c in &rc.closure {
            if !m.keys().any(|(n, _)| *n == c.crate_name) {
                r.refusals.push(format!("(d1) floor: the {which} tree's touched build has no file for closure crate {}", c.crate_name));
            }
        }
    }
    let keys: BTreeSet<&(String, bool)> = np.keys().chain(nt.keys()).collect();
    for k in keys {
        let (a, b) = (np.get(k), nt.get(k));
        let same = a.is_some() && a == b;
        let detail = if same { String::new() } else { format!("pre {:?} tip {:?}", a.map(|s| s.iter().collect::<Vec<_>>()), b.map(|s| s.iter().collect::<Vec<_>>())) };
        r.d1.push(Cmp { item: k.0.clone(), host: k.1, same, detail, words: Vec::new() });
    }
    // (d2): every workspace closure crate's line in the touched build, after the two substitutions.
    let lines = |rc: &Receipt, krate: &str| -> Vec<String> {
        let mut v: Vec<String> = rc.running.iter().filter(|(t, c, _)| t == TOUCHED && c == krate).map(|(_, _, l)| normalize_d2(l, &rc.target, &rc.root)).collect();
        v.sort();
        v
    };
    for c in pre.closure.iter().filter(|c| c.workspace) {
        let (a, b) = (lines(pre, &c.crate_name), lines(tip, &c.crate_name));
        if a.is_empty() || b.is_empty() {
            r.refusals.push(format!("(d2) floor: no `Running` line for workspace closure crate {} in the {} touched build", c.crate_name, if a.is_empty() { "pre" } else { "tip" }));
            continue;
        }
        let same = a == b;
        let (pre_only, tip_only) = (word_difference(&a.join(" "), &b.join(" ")), word_difference(&b.join(" "), &a.join(" ")));
        let detail = if same { String::new() } else { format!("pre-only words {pre_only:?}; tip-only words {tip_only:?}") };
        let words = if same { Vec::new() } else { pre_only.into_iter().chain(tip_only).collect() };
        r.d2.push(Cmp { item: c.crate_name.clone(), host: false, same, detail, words });
    }
    // (d3): the final lines, runner with runner and probe with probe.
    for tag in ["runner", "probe"] {
        match (pre.final_of(tag), tip.final_of(tag)) {
            (Some(a), Some(b)) => match (normalize_d3(a, &pre.target, &pre.root), normalize_d3(b, &tip.target, &tip.root)) {
                (Ok(na), Ok(nb)) => {
                    let same = na == nb;
                    let (pre_only, tip_only) = (word_difference(&na, &nb), word_difference(&nb, &na));
                    let detail = if same { String::new() } else { format!("pre-only words {pre_only:?}; tip-only words {tip_only:?}") };
                    let words = if same { Vec::new() } else { pre_only.into_iter().chain(tip_only).collect() };
                    r.d3.push(Cmp { item: tag.to_owned(), host: false, same, detail, words });
                }
                (Err(e), _) | (_, Err(e)) => r.d3.push(Cmp { item: tag.to_owned(), host: false, same: false, detail: e.to_string(), words: Vec::new() }),
            },
            _ => r.refusals.push(format!("(d3) a receipt has no `{tag}` final line")),
        }
    }
    // (d4): the environment and the normalised config and lock.
    let rustc = |rc: &Receipt| rc.rustc_vv.join(" / ");
    for (item, a, b) in [
        ("BOYKO_PROFILE", pre.boyko_profile.clone(), tip.boyko_profile.clone()),
        ("rustc -vV", rustc(pre), rustc(tip)),
        ("receipt format", RECEIPT_FORMAT.to_owned(), RECEIPT_FORMAT.to_owned()),
        (".cargo/config.toml sha256", pre.config_sha256.clone(), tip.config_sha256.clone()),
        ("Cargo.lock sha256", pre.lock_sha256.clone(), tip.lock_sha256.clone()),
    ] {
        let same = a == b;
        r.d4.push(Cmp { item: item.to_owned(), host: false, same, detail: if same { String::new() } else { format!("pre {a} tip {b}") }, words: Vec::new() });
    }
    r
}
