//! V11 G-UNIT's pure parts (IB design D14 as patched by Rev 5, Rev 6 and ruling 15): the closed
//! normalisation list of (d2)/(d3), (f)'s diagnostic match, and the `ug15-ib-receipt` format.
//!
//! # The normalisation list is closed (Rev 6 (d), O13, O17)
//!
//! * (d2), a workspace closure crate's `Running` line: exactly two substitutions, each in both path
//!   separator forms — the tree's `CARGO_TARGET_DIR` → `<T>` and its root → `<R>`, the LONGER string
//!   first (O17: when the target dir lies under the root, root-first would leave `<R>/target` on one
//!   side and `<T>` on the other). Nothing is removed: `-C metadata`, `-C extra-filename` and every
//!   `--extern` file name are tree-independent (cargo hashes a path package's id relative to the
//!   workspace root), so they must match.
//! * (d3), a final unit's line: (d2), then exactly these: `-C metadata=…` and `-C extra-filename=…`
//!   dropped (they hash the enabled pin feature); `ib-pin-*` dropped from `--cfg feature=` and from the
//!   `--check-cfg` values; the one `-C link-arg=/MAP:…` value → `<MAP>` (the retain directory, per
//!   tree and run by design). Each side must carry exactly one `/MAP:` token, or RED.
//!
//! Nothing else is normalised. A difference the list does not name stays a difference.

use crate::red::{Red, RedKind, Result};

/// The receipt format line, compared by (d4).
pub const RECEIPT_FORMAT: &str = "ug15-ib-receipt v1";

/// The diagnostic (f) requires on every runner build: rustc's `unused_crate_dependencies` for the
/// host, which fires exactly when no path names the extern, i.e. when rustc never loads it.
pub const F_NEEDLE: &str = "extern crate `boyko_app` is unused in crate `jolt_parity_pyramid`";

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
            if line.is_empty() {
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
