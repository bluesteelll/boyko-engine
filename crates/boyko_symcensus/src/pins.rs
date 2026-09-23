//! Leg (2), the asm pins (03 §6): capture on B3's tree, check on every later rung.
//!
//! **Capture** builds each subject under `release` (probe (iv): reproducible, so no deterministic
//! profile), locates every cut §4.2 candidate by its rule, and freezes the pin list BY NAME: a
//! generic candidate's instantiation chosen by the rule at capture is the one checked forever
//! after (cut D5). A candidate with no defined code symbol is written to the absent list with its
//! reason and is never pinned (03 §6 "Missing symbols").
//!
//! **One name, several copies** (PC-23). Fat LTO can leave several local copies of one
//! instantiation — `register_new::<Transform>` has three in `clear`, one per instantiating crate,
//! and the v0 demangler drops the instantiating crate, so the copies share a name. A pin is
//! therefore (subject, normalised name) → the MULTISET of its copies' normalised bodies: equal
//! copies collapse to one body with `copies=k`; unequal copies are one entry each. Raw names are
//! never used to order copies, because the disambiguator hash in them moves with `-C metadata`.
//!
//! **Check** re-locates each pinned name. A name with no defined code symbol is RED
//! [`RedKind::SymbolAbsent`] — "absent", never "moved by 0" (P29: a missing symbol means
//! somebody removed an attribute; critique W1). A body whose normalised text differs is "moved":
//! RED unless the rung names it (attributed mode, 02 §2's table).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::candidates::{self, CANDIDATES, Candidate};
use crate::llvm::Sym;
use crate::normalize::{self, RenameList};
use crate::objbuild::{self, Built, Ctx, Request, SUBJECTS, Subject};
use crate::objview::{Llvm, ObjView};
use crate::red::{Red, RedKind, Result};
use crate::sha256;

/// The first line of every pin file.
pub const MAGIC: &str = "# ug15 leg2 pins v1";

/// The profile leg (2) reads (probe (iv) found it reproducible).
pub const PROFILE: &str = "release";

/// One pinned body (one distinct normalised body of one name in one subject).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The subject key.
    pub subject: String,
    /// The cut §4.2 candidate id.
    pub cand: String,
    /// The normalised demangled name.
    pub name: String,
    /// The section's `RawDataSize` (probe (ii)).
    pub size: u64,
    /// How many copies of the name carry this exact body.
    pub copies: usize,
    /// sha256 of [`Pin::body`].
    pub sha256: String,
    /// The normalised body text.
    pub body: String,
}

impl Pin {
    fn header(&self) -> String {
        format!(
            "== {} | {} | {} | size={} (section RawDataSize) | copies={} | sha256={}",
            self.subject, self.cand, self.name, self.size, self.copies, self.sha256
        )
    }
}

/// An absent candidate: recorded, never pinned.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Absent {
    /// The candidate id.
    pub cand: String,
    /// The subject it was looked for in.
    pub subject: String,
    /// Why it is absent, and its carrier when map (a) names one.
    pub reason: String,
}

/// A parsed pin file.
#[derive(Clone, Debug, Default)]
pub struct PinFile {
    /// `# …` provenance lines.
    pub header: Vec<String>,
    /// The pins, in file order.
    pub pins: Vec<Pin>,
}

impl PinFile {
    /// The file text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(MAGIC);
        s.push('\n');
        for h in &self.header {
            let _ = writeln!(s, "# {h}");
        }
        for p in &self.pins {
            s.push_str(&p.header());
            s.push('\n');
            s.push_str(&p.body);
        }
        s
    }

    /// Parses a pin file; RED if a body's sha256 is not the one its header records (a hand edit).
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.split_inclusive('\n');
        if lines.next().map(str::trim_end) != Some(MAGIC) {
            return Err(Red::new(RedKind::Malformed, format!("not a pin file: the first line is not `{MAGIC}`")));
        }
        let mut f = Self::default();
        let mut cur: Option<Pin> = None;
        for line in lines {
            if let Some(h) = line.strip_prefix("== ") {
                if let Some(p) = cur.take() {
                    f.pins.push(p);
                }
                let parts: Vec<&str> = h.trim_end().split(" | ").collect();
                let bad = || Red::new(RedKind::Malformed, format!("pin header not understood: {}", line.trim_end()));
                if parts.len() != 6 {
                    return Err(bad());
                }
                let size = parts[3].strip_prefix("size=").and_then(|v| v.split(' ').next()).and_then(|v| v.parse().ok()).ok_or_else(bad)?;
                let copies = parts[4].strip_prefix("copies=").and_then(|v| v.parse().ok()).ok_or_else(bad)?;
                let sha = parts[5].strip_prefix("sha256=").ok_or_else(bad)?;
                cur = Some(Pin {
                    subject: parts[0].to_owned(),
                    cand: parts[1].to_owned(),
                    name: parts[2].to_owned(),
                    size,
                    copies,
                    sha256: sha.to_owned(),
                    body: String::new(),
                });
            } else if let Some(p) = cur.as_mut() {
                p.body.push_str(line);
            } else if let Some(h) = line.strip_prefix("# ") {
                f.header.push(h.trim_end().to_owned());
            } else {
                return Err(Red::new(RedKind::Malformed, format!("pin file line outside any pin: {}", line.trim_end())));
            }
        }
        if let Some(p) = cur.take() {
            f.pins.push(p);
        }
        for p in &f.pins {
            let got = sha256::bytes_hex(p.body.as_bytes());
            if got != p.sha256 {
                return Err(Red::new(
                    RedKind::Malformed,
                    format!("pin {} {} in {}: the body hashes to {got}, the header says {} (the body was edited by hand)", p.cand, p.name, p.subject, p.sha256),
                ));
            }
        }
        Ok(f)
    }
}

/// The directory of the committed UG-15 data.
#[must_use]
pub fn data_dir(root: &Path) -> PathBuf {
    root.join("crates").join("boyko_symcensus").join("ug15")
}

/// `pins/leg2-<subject>.pins`.
#[must_use]
pub fn pin_path(root: &Path, subject: &str) -> PathBuf {
    data_dir(root).join("pins").join(format!("leg2-{subject}.pins"))
}

/// `pins/leg2-absent.txt`.
#[must_use]
pub fn absent_path(root: &Path) -> PathBuf {
    data_dir(root).join("pins").join("leg2-absent.txt")
}

/// Reads the committed pin file of `subject`, if there is one.
pub fn read_pins(root: &Path, subject: &str) -> Result<Option<PinFile>> {
    let p = pin_path(root, subject);
    match std::fs::read_to_string(&p) {
        Ok(t) => PinFile::parse(&t).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Red::io(&p, &e)),
    }
}

/// The defined CODE symbols whose normalised name is `name` (funclets excluded; they come with
/// their owner's section).
#[must_use]
pub fn by_name<'a>(view: &'a ObjView, name: &str) -> Vec<&'a Sym> {
    view.syms
        .defined
        .iter()
        .filter(|s| candidates::is_code(s.class) && !s.raw.starts_with('?') && normalize::norm_name(&s.name) == name)
        .collect()
}

/// The candidate's located symbols in `view`, grouped by normalised name.
fn locate_all<'a>(view: &'a ObjView, cand: &Candidate) -> BTreeMap<String, Vec<&'a Sym>> {
    let mut out: BTreeMap<String, Vec<&'a Sym>> = BTreeMap::new();
    for rule in cand.rules {
        for s in candidates::locate(&view.syms.defined, *rule) {
            let name = normalize::norm_name(&s.name);
            // A `FirstWithPrefix` rule picks one symbol; its NAME is what is frozen, so every copy
            // of that name joins the pin.
            for copy in by_name(view, &name) {
                let v = out.entry(name.clone()).or_default();
                if !v.iter().any(|x| x.raw == copy.raw) {
                    v.push(copy);
                }
            }
        }
    }
    out
}

/// The pins of one (candidate or pinned) name: one entry per distinct normalised body.
fn pins_of(llvm: &Llvm, view: &ObjView, subject: &str, cand: &str, name: &str, syms: &[&Sym], rename: &RenameList) -> Result<Vec<Pin>> {
    let mut by_sha: BTreeMap<String, Pin> = BTreeMap::new();
    for s in syms {
        let b = view.body(&llvm.objdump, &s.raw, rename)?;
        by_sha
            .entry(b.sha256.clone())
            .and_modify(|p| p.copies += 1)
            .or_insert(Pin { subject: subject.to_owned(), cand: cand.to_owned(), name: name.to_owned(), size: b.section_len, copies: 1, sha256: b.sha256, body: b.text });
    }
    Ok(by_sha.into_values().collect())
}

fn build_view(ctx: &Ctx, llvm: &Llvm, s: Subject) -> Result<(Built, ObjView)> {
    let built = objbuild::build(ctx, &Request::new(s, PROFILE, &[]))?;
    let view = ObjView::load(llvm, &built.object)?;
    Ok((built, view))
}

/// The provenance header of a pin file.
fn provenance(ctx: &Ctx, llvm: &Llvm, built: &Built) -> Vec<String> {
    let mut h: Vec<String> = crate::probe::header(ctx, llvm, &format!("leg (2) pins of {} under {PROFILE}", built.subject.key)).lines().map(str::to_owned).collect();
    h.extend(built.receipt().lines().map(str::to_owned));
    h
}

/// Leg (2) capture: writes `pins/leg2-<subject>.pins` for every subject with a candidate and
/// `pins/leg2-absent.txt`; returns the receipt. RED if a subject yields no pin at all.
pub fn capture(ctx: &Ctx, llvm: &Llvm) -> Result<String> {
    let mut out = crate::probe::header(ctx, llvm, "leg (2) capture");
    let mut absent: Vec<Absent> = Vec::new();
    for s in SUBJECTS {
        let cands: Vec<&Candidate> = CANDIDATES.iter().filter(|c| c.subjects.contains(&s.key)).collect();
        if cands.is_empty() {
            continue;
        }
        let (built, view) = build_view(ctx, llvm, s)?;
        let mut file = PinFile { header: provenance(ctx, llvm, &built), pins: Vec::new() };
        for c in cands {
            let found = locate_all(&view, c);
            if found.is_empty() {
                absent.push(Absent {
                    cand: c.id.to_owned(),
                    subject: s.key.to_owned(),
                    reason: format!(
                        "no defined code symbol matches {:?} in the post-LTO object: every instantiation was inlined into its callers or removed by fat LTO",
                        c.rules
                    ),
                });
                let _ = writeln!(out, "{} {}: ABSENT", s.key, c.id);
                continue;
            }
            for (name, syms) in found {
                let pins = pins_of(llvm, &view, s.key, c.id, &name, &syms, &RenameList::default())?;
                for p in &pins {
                    let _ = writeln!(out, "{} {}: PINNED {} size={} copies={} sha={}", s.key, c.id, p.name, p.size, p.copies, &p.sha256[..16]);
                }
                file.pins.extend(pins);
            }
        }
        built.check_intact()?;
        if file.pins.is_empty() {
            return Err(Red::new(RedKind::EmptyCensus, format!("leg (2): subject {} has candidates and no pin at all", s.key)));
        }
        let p = pin_path(&ctx.root, s.key);
        write(&p, &file.to_text())?;
        let _ = writeln!(out, "{}: {} pin(s) -> {}", s.key, file.pins.len(), p.display());
    }
    absent.sort();
    write(&absent_path(&ctx.root), &absent_text(&absent))?;
    let _ = writeln!(out, "absent candidates: {}", absent.len());
    for a in &absent {
        let _ = writeln!(out, "  {} in {}: {}", a.cand, a.subject, a.reason);
    }
    Ok(out)
}

/// The text of the absent list: `cand\tsubject\treason`.
#[must_use]
pub fn absent_text(rows: &[Absent]) -> String {
    let mut s = String::from("# UG-15 leg (2): candidates with no symbol in the post-LTO object, recorded at capture and never pinned (03 §6 \"Missing symbols\").\n# cand\tsubject\treason\n");
    for a in rows {
        let _ = writeln!(s, "{}\t{}\t{}", a.cand, a.subject, a.reason);
    }
    s
}

/// Parses the absent list.
pub fn parse_absent(text: &str) -> Result<Vec<Absent>> {
    let mut v = Vec::new();
    for l in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut it = l.splitn(3, '\t');
        let (Some(c), Some(s), Some(r)) = (it.next(), it.next(), it.next()) else {
            return Err(Red::new(RedKind::Malformed, format!("absent list line not understood: {l}")));
        };
        v.push(Absent { cand: c.to_owned(), subject: s.to_owned(), reason: r.to_owned() });
    }
    Ok(v)
}

/// A named (attributed-mode) move: `<pin name or candidate id>\t<reason>` per line.
pub fn parse_named(text: &str) -> Result<Vec<(String, String)>> {
    let mut v = Vec::new();
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (k, r) = l.split_once('\t').ok_or_else(|| Red::new(RedKind::Malformed, format!("named-move line needs `<pin>\\t<reason>`: {l}")))?;
        if r.trim().is_empty() {
            return Err(Red::new(RedKind::Malformed, format!("named move {k} carries no reason")));
        }
        v.push((k.trim().to_owned(), r.trim().to_owned()));
    }
    Ok(v)
}

fn write(p: &Path, text: &str) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| Red::io(d, &e))?;
    }
    std::fs::write(p, text).map_err(|e| Red::io(p, &e))
}

/// The pins of `file` grouped by (name) with the multiset of (sha256, size, copies).
fn multiset(pins: &[Pin]) -> BTreeMap<String, BTreeSet<(String, u64, usize)>> {
    let mut m: BTreeMap<String, BTreeSet<(String, u64, usize)>> = BTreeMap::new();
    for p in pins {
        m.entry(p.name.clone()).or_default().insert((p.sha256.clone(), p.size, p.copies));
    }
    m
}

/// Leg (2) check against the committed pins, under an optional rename list (old → new) and
/// named moves. Returns the receipt; RED on an absent pinned name or an unnamed move.
pub fn check(ctx: &Ctx, llvm: &Llvm, rename: &RenameList, named: &[(String, String)], subjects: &[Subject]) -> Result<String> {
    let mut out = crate::probe::header(ctx, llvm, "leg (2) check");
    let _ = writeln!(out, "rename entries {}, named moves {}", rename.0.len(), named.len());
    let mut absent: Vec<String> = Vec::new();
    let mut moved: Vec<String> = Vec::new();
    let mut named_moved: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for &s in subjects {
        let Some(file) = read_pins(&ctx.root, s.key)? else { continue };
        let (built, view) = build_view(ctx, llvm, s)?;
        let mut by_name_pins: BTreeMap<String, Vec<Pin>> = BTreeMap::new();
        for p in &file.pins {
            by_name_pins.entry(p.name.clone()).or_default().push(p.clone());
        }
        for (old_name, old_pins) in &by_name_pins {
            checked += old_pins.len();
            let cand = old_pins[0].cand.clone();
            let name = rename.apply(old_name);
            let syms = by_name(&view, &name);
            if syms.is_empty() {
                absent.push(format!("{} {} `{name}` (pinned as `{old_name}`)", s.key, cand));
                let _ = writeln!(out, "{} {}: ABSENT {name}", s.key, cand);
                continue;
            }
            // The committed bodies, mapped through the rename list, against the fresh bodies.
            let old_mapped: Vec<Pin> = old_pins
                .iter()
                .map(|p| {
                    let body: String = p.body.split_inclusive('\n').map(|l| rename.apply(l)).collect();
                    Pin { name: name.clone(), sha256: sha256::bytes_hex(body.as_bytes()), body, ..p.clone() }
                })
                .collect();
            let fresh = pins_of(llvm, &view, s.key, &cand, &name, &syms, &RenameList::default())?;
            let (a, b) = (multiset(&old_mapped), multiset(&fresh));
            if a == b {
                let _ = writeln!(out, "{} {}: identical {name}", s.key, cand);
                continue;
            }
            let line = format!(
                "{} {} `{name}`: pinned {:?} → now {:?}",
                s.key,
                cand,
                a.values().flatten().map(|(h, z, k)| format!("{}:{z}B×{k}", &h[..12])).collect::<Vec<_>>(),
                b.values().flatten().map(|(h, z, k)| format!("{}:{z}B×{k}", &h[..12])).collect::<Vec<_>>()
            );
            let why = named.iter().find(|(k, _)| *k == cand || *k == name || *k == *old_name);
            match why {
                Some((_, r)) => {
                    let _ = writeln!(out, "MOVED (named: {r}) {line}");
                    named_moved.push(line);
                }
                None => {
                    let _ = writeln!(out, "MOVED {line}");
                    for f in &fresh {
                        let _ = writeln!(out, "---- fresh body {} ({} B):\n{}", &f.sha256[..16], f.size, f.body);
                    }
                    moved.push(line);
                }
            }
        }
        built.check_intact()?;
    }
    let _ = writeln!(out, "checked {checked} pin(s): {} absent, {} moved unnamed, {} moved named", absent.len(), moved.len(), named_moved.len());
    if checked == 0 {
        return Err(Red::new(RedKind::EmptyCensus, format!("{out}leg (2) found no committed pin to check")));
    }
    if !absent.is_empty() {
        return Err(Red::new(RedKind::SymbolAbsent, format!("{out}leg (2) RED: pinned symbol(s) absent: {absent:?}")));
    }
    if !moved.is_empty() {
        return Err(Red::new(RedKind::Mismatch, format!("{out}leg (2) RED: {} unnamed move(s)", moved.len())));
    }
    Ok(out)
}
