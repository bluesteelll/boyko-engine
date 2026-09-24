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
//!
//! **The frozen set is closed** (B3 review W1). A pin that is no longer in the committed data is
//! never checked, so a deleted pin file, or a pin block cut out with its body, would otherwise read
//! as a smaller green. Capture therefore writes `pins/leg2-frozen.tsv`, one row per pinned body,
//! beside the pin files, and every reader of a pin file goes through [`read_pins`], which REDs
//! [`RedKind::PinSet`] unless (1) every (candidate, subject) pair of [`CANDIDATES`] is either in
//! the frozen list or in `pins/leg2-absent.txt`, never both and never neither, and (2) the
//! subject's pin file holds exactly its frozen rows. The row set is compared, not a count, so a
//! multi-name candidate (H-1, H-2, H-3) cannot lose one name while it keeps another.
//!
//! **The dispositions are re-verified against the object** (B3 retest R1). The committed data is
//! only claims until an object confirms them: a consistent edit of all three files can move a
//! pinned pair into the absent list, or cut one name of a multi-name candidate from both the pin
//! file and the frozen list, and the pre-build read cannot tell either from what capture wrote.
//! So [`check`] builds every subject the candidate list names and, per subject, REDs
//! [`RedKind::PinSet`] on each [`contradicted`] disposition: a pair recorded absent whose rules
//! now locate a symbol ("recorded absent, but present" — also a real codegen change, a candidate
//! that fat LTO used to inline away and no longer does), and a name that a pinned candidate's
//! `Exact` or `Contains` rule locates but that is not pinned ("located, but not frozen").
//! `FirstWithPrefix` rules take no part in the second test: cut D5 freezes the instantiation the
//! rule chose at capture by NAME, so a re-run of the rule that picks another instantiation is not
//! a fault, and the one frozen name is already checked by name (RED [`RedKind::SymbolAbsent`] if
//! it goes). A contradiction is never a named move: `--named` attributes body changes, not the set.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::candidates::{self, CANDIDATES, Candidate, Rule};
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
    ///
    /// Line endings are normalised to LF first: the digests are taken over LF text, and a Windows
    /// checkout with `core.autocrlf = true` (this repository's default) writes the committed LF
    /// blob back as CRLF, which would otherwise read as an edit of every body.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.replace("\r\n", "\n");
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

/// `pins/leg2-frozen.tsv`.
#[must_use]
pub fn frozen_path(root: &Path) -> PathBuf {
    data_dir(root).join("pins").join("leg2-frozen.tsv")
}

/// One row of the frozen list: one pinned body, as capture wrote it into its pin file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Frozen {
    /// The subject key.
    pub subject: String,
    /// The candidate id.
    pub cand: String,
    /// The normalised demangled name.
    pub name: String,
    /// The section's `RawDataSize`.
    pub size: u64,
    /// How many copies carry this body.
    pub copies: usize,
    /// sha256 of the normalised body.
    pub sha256: String,
}

impl Frozen {
    /// The frozen row of `pin`.
    #[must_use]
    pub fn of(pin: &Pin) -> Self {
        Self {
            subject: pin.subject.clone(),
            cand: pin.cand.clone(),
            name: pin.name.clone(),
            size: pin.size,
            copies: pin.copies,
            sha256: pin.sha256.clone(),
        }
    }
}

/// The text of the frozen list: `subject\tcand\tname\tsize\tcopies\tsha256`, in the order given.
#[must_use]
pub fn frozen_text(rows: &[Frozen]) -> String {
    let mut s = String::from(
        "# UG-15 leg (2): the pin set capture froze, one row per pinned body (B3 review W1). Written by\n\
         # `ug15 leg2 capture` beside the pin files; every reader of a pin file REDs [pin set] unless the\n\
         # file holds exactly its subject's rows here, and unless every (candidate, subject) pair of the\n\
         # candidate list is either here or in leg2-absent.txt.\n\
         # subject\tcand\tname\tsize\tcopies\tsha256\n",
    );
    for r in rows {
        let _ = writeln!(s, "{}\t{}\t{}\t{}\t{}\t{}", r.subject, r.cand, r.name, r.size, r.copies, r.sha256);
    }
    s
}

/// Parses the frozen list.
pub fn parse_frozen(text: &str) -> Result<Vec<Frozen>> {
    let mut v = Vec::new();
    for l in text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = l.split('\t').collect();
        let bad = || Red::new(RedKind::Malformed, format!("frozen-list line not understood: {l}"));
        let [subject, cand, name, size, copies, sha] = f.as_slice() else { return Err(bad()) };
        v.push(Frozen {
            subject: (*subject).to_owned(),
            cand: (*cand).to_owned(),
            name: (*name).to_owned(),
            size: size.parse().map_err(|_| bad())?,
            copies: copies.parse().map_err(|_| bad())?,
            sha256: (*sha).to_owned(),
        });
    }
    Ok(v)
}

/// Reads a committed file that must exist: a missing one is RED [`RedKind::PinSet`], because the
/// data it holds is what makes the frozen set closed.
fn read_required(p: &Path, what: &str) -> Result<String> {
    match std::fs::read_to_string(p) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(Red::new(RedKind::PinSet, format!("{} is missing: {what}", p.display())))
        }
        Err(e) => Err(Red::io(p, &e)),
    }
}

/// The committed frozen list.
pub fn read_frozen(root: &Path) -> Result<Vec<Frozen>> {
    parse_frozen(&read_required(&frozen_path(root), "the frozen list says which pins capture froze, so without it a lost pin cannot be told from one never taken")?)
}

/// The committed absent list.
pub fn read_absent(root: &Path) -> Result<Vec<Absent>> {
    parse_absent(&read_required(&absent_path(root), "the absent list is the other half of every candidate's disposition")?)
}

/// RED [`RedKind::PinSet`] unless every (candidate, subject) pair of [`CANDIDATES`] is either
/// frozen (at least one row) or recorded absent, never both and never neither, and unless every
/// frozen or absent row names such a pair.
pub fn verify_closure(frozen: &[Frozen], absent: &[Absent]) -> Result<()> {
    let mut faults: Vec<String> = Vec::new();
    for c in CANDIDATES {
        for s in c.subjects {
            let pinned = frozen.iter().any(|f| f.cand == c.id && f.subject == *s);
            let gone = absent.iter().any(|a| a.cand == c.id && a.subject == *s);
            match (pinned, gone) {
                (true, true) => faults.push(format!("{} in {s} is both frozen and recorded absent", c.id)),
                (false, false) => faults.push(format!("{} in {s} is neither frozen nor recorded absent", c.id)),
                _ => {}
            }
        }
    }
    let known = |cand: &str, subject: &str| CANDIDATES.iter().any(|c| c.id == cand && c.subjects.contains(&subject));
    for f in frozen.iter().filter(|f| !known(&f.cand, &f.subject)) {
        faults.push(format!("frozen row {} `{}` in {}: the candidate list names no such pair", f.cand, f.name, f.subject));
    }
    for a in absent.iter().filter(|a| !known(&a.cand, &a.subject)) {
        faults.push(format!("absent row {} in {}: the candidate list names no such pair", a.cand, a.subject));
    }
    if faults.is_empty() {
        Ok(())
    } else {
        Err(Red::new(RedKind::PinSet, format!("leg (2)'s committed data is not closed over the candidate list: {faults:?}")))
    }
}

/// RED [`RedKind::PinSet`] unless `file` (the parsed pin file of `subject`, `None` when it does
/// not exist) holds exactly the frozen rows of `subject`: no file where rows are frozen, no row
/// missing, none added.
pub fn verify_file(subject: &str, file: Option<&PinFile>, frozen: &[Frozen]) -> Result<()> {
    let mut want: Vec<Frozen> = frozen.iter().filter(|f| f.subject == subject).cloned().collect();
    let Some(file) = file else {
        return if want.is_empty() {
            Ok(())
        } else {
            Err(Red::new(RedKind::PinSet, format!("leg2-{subject}.pins is missing and the frozen list holds {} pin(s) of {subject}", want.len())))
        };
    };
    let mut got: Vec<Frozen> = file.pins.iter().map(Frozen::of).collect();
    want.sort();
    got.sort();
    if got == want {
        return Ok(());
    }
    // The difference is taken as MULTISETS (B3 retest R2): a duplicated block is a row the file
    // holds once more than the frozen list, and a membership test would name nothing.
    let mut net: BTreeMap<&Frozen, i64> = BTreeMap::new();
    for g in &got {
        *net.entry(g).or_default() += 1;
    }
    for w in &want {
        *net.entry(w).or_default() -= 1;
    }
    let row = |f: &Frozen, n: i64| {
        let times = if n.abs() > 1 { format!(" ×{}", n.abs()) } else { String::new() };
        format!("{} `{}` {} (subject {}){times}", f.cand, f.name, &f.sha256[..f.sha256.len().min(12)], f.subject)
    };
    let missing: Vec<String> = net.iter().filter(|(_, n)| **n < 0).map(|(f, n)| row(f, *n)).collect();
    let extra: Vec<String> = net.iter().filter(|(_, n)| **n > 0).map(|(f, n)| row(f, *n)).collect();
    Err(Red::new(
        RedKind::PinSet,
        format!("leg2-{subject}.pins holds {} pin(s), the frozen list {}: frozen but not in the file {missing:?}; in the file but not frozen {extra:?}", got.len(), want.len()),
    ))
}

/// Reads the committed pin file of `subject` and proves it is the one capture froze: RED
/// [`RedKind::PinSet`] on any break of [`verify_closure`] or [`verify_file`]. `Ok(None)` only when
/// the frozen list holds no row of `subject` and no file exists.
pub fn read_pins(root: &Path, subject: &str) -> Result<Option<PinFile>> {
    let frozen = read_frozen(root)?;
    verify_closure(&frozen, &read_absent(root)?)?;
    let p = pin_path(root, subject);
    let file = match std::fs::read_to_string(&p) {
        Ok(t) => Some(PinFile::parse(&t)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Red::io(&p, &e)),
    };
    verify_file(subject, file.as_ref(), &frozen)?;
    Ok(file)
}

/// The defined CODE symbols whose normalised name is `name` (funclets excluded; they come with
/// their owner's section).
#[must_use]
pub fn by_name<'a>(view: &'a ObjView, name: &str) -> Vec<&'a Sym> {
    by_name_in(&view.syms.defined, name)
}

fn by_name_in<'a>(defined: &'a [Sym], name: &str) -> Vec<&'a Sym> {
    defined.iter().filter(|s| candidates::is_code(s.class) && !s.raw.starts_with('?') && normalize::norm_name(&s.name) == name).collect()
}

/// The candidate's located symbols among `defined`, grouped by normalised name.
fn locate_all<'a>(defined: &'a [Sym], cand: &Candidate) -> BTreeMap<String, Vec<&'a Sym>> {
    let mut out: BTreeMap<String, Vec<&'a Sym>> = BTreeMap::new();
    for rule in cand.rules {
        for s in candidates::locate(defined, *rule) {
            let name = normalize::norm_name(&s.name);
            // A `FirstWithPrefix` rule picks one symbol; its NAME is what is frozen, so every copy
            // of that name joins the pin.
            for copy in by_name_in(defined, &name) {
                let v = out.entry(name.clone()).or_default();
                if !v.iter().any(|x| x.raw == copy.raw) {
                    v.push(copy);
                }
            }
        }
    }
    out
}

/// The dispositions of `subject` that its built object contradicts, one sentence each (empty when
/// the object confirms them all). `defined` is the object's defined symbol table, `file` the
/// subject's verified pin file (empty when every pair of the subject is recorded absent), and
/// `rename` the check's rename list, through which the pinned names are read.
///
/// - **recorded absent, but present**: an absent row of `subject` whose candidate's rules locate
///   at least one defined code symbol.
/// - **located, but not frozen**: a name that an `Exact` or `Contains` rule of a candidate pinned
///   in `subject` locates, and that is none of that candidate's pinned names. `FirstWithPrefix`
///   rules are skipped here (cut D5; see the module doc).
///
/// A row naming no candidate is not reported here: [`verify_closure`] has already REDed it.
#[must_use]
pub fn contradicted(subject: &str, defined: &[Sym], file: &PinFile, absent: &[Absent], rename: &RenameList) -> Vec<String> {
    let mut faults: Vec<String> = Vec::new();
    for a in absent.iter().filter(|a| a.subject == subject) {
        let Some(c) = CANDIDATES.iter().find(|c| c.id == a.cand) else { continue };
        let found = locate_all(defined, c);
        if !found.is_empty() {
            faults.push(format!("recorded absent, but present: {} in {subject}: {:?}", c.id, found.keys().collect::<Vec<_>>()));
        }
    }
    for c in CANDIDATES.iter().filter(|c| c.subjects.contains(&subject)) {
        let pinned: BTreeSet<String> = file.pins.iter().filter(|p| p.cand == c.id).map(|p| rename.apply(&p.name)).collect();
        if pinned.is_empty() {
            continue;
        }
        let mut extra: BTreeSet<String> = BTreeSet::new();
        for rule in c.rules.iter().filter(|r| !matches!(r, Rule::FirstWithPrefix(_))) {
            extra.extend(candidates::locate(defined, *rule).into_iter().map(|s| normalize::norm_name(&s.name)).filter(|n| !pinned.contains(n)));
        }
        for name in extra {
            faults.push(format!("located, but not frozen: {} in {subject}: `{name}`", c.id));
        }
    }
    faults
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
    let mut h: Vec<String> = crate::probes::header(ctx, llvm, &format!("leg (2) pins of {} under {PROFILE}", built.subject.key)).lines().map(str::to_owned).collect();
    h.extend(built.receipt().lines().map(str::to_owned));
    h
}

/// Leg (2) capture: writes `pins/leg2-<subject>.pins` for every subject with a candidate,
/// `pins/leg2-absent.txt` and `pins/leg2-frozen.tsv`; returns the receipt. RED if a subject yields
/// no pin at all, or if the written data is not closed over the candidate list.
pub fn capture(ctx: &Ctx, llvm: &Llvm) -> Result<String> {
    let mut out = crate::probes::header(ctx, llvm, "leg (2) capture");
    let mut absent: Vec<Absent> = Vec::new();
    let mut frozen: Vec<Frozen> = Vec::new();
    for s in SUBJECTS {
        let cands: Vec<&Candidate> = CANDIDATES.iter().filter(|c| c.subjects.contains(&s.key)).collect();
        if cands.is_empty() {
            continue;
        }
        let (built, view) = build_view(ctx, llvm, s)?;
        let mut file = PinFile { header: provenance(ctx, llvm, &built), pins: Vec::new() };
        for c in cands {
            let found = locate_all(&view.syms.defined, c);
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
        frozen.extend(file.pins.iter().map(Frozen::of));
        let _ = writeln!(out, "{}: {} pin(s) -> {}", s.key, file.pins.len(), p.display());
    }
    absent.sort();
    verify_closure(&frozen, &absent)?;
    write(&absent_path(&ctx.root), &absent_text(&absent))?;
    let fp = frozen_path(&ctx.root);
    write(&fp, &frozen_text(&frozen))?;
    let _ = writeln!(out, "frozen: {} pin(s) -> {}", frozen.len(), fp.display());
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

/// Where leg (2)'s objects come from. [`check`] builds each subject and reads its post-LTO
/// object; the in-tree sweep (`tests/ug15_leg2_pin_set_closure.rs`) supplies a model object. Both
/// run the one per-subject step, [`check_with`], so the sweep exercises the gate's own code rather
/// than a copy of its loop (B3 close test T1: a copy stayed green when `check` stopped calling
/// [`contradicted`]).
pub trait Objects {
    /// One subject's object.
    type Object;

    /// The object of `s`; [`check`] builds it now, under [`PROFILE`].
    fn object(&self, s: Subject) -> Result<Self::Object>;

    /// The object's defined symbol table.
    fn defined<'o>(&self, o: &'o Self::Object) -> &'o [Sym];

    /// The bodies of `name` in `o`, one pin per distinct normalised body. `syms` are the defined
    /// code symbols of that name (every copy), as [`check_with`] located them in [`Objects::defined`].
    fn bodies(&self, o: &Self::Object, subject: &str, cand: &str, name: &str, syms: &[&Sym]) -> Result<Vec<Pin>>;

    /// Called once the subject's pins are compared: RED if `o` changed while it was read.
    fn done(&self, o: &Self::Object) -> Result<()>;
}

/// The objects [`check`] reads: each subject built under [`PROFILE`] by this invocation.
struct BuiltObjects<'a> {
    ctx: &'a Ctx,
    llvm: &'a Llvm,
}

impl Objects for BuiltObjects<'_> {
    type Object = (Built, ObjView);

    fn object(&self, s: Subject) -> Result<Self::Object> {
        build_view(self.ctx, self.llvm, s)
    }

    fn defined<'o>(&self, o: &'o Self::Object) -> &'o [Sym] {
        &o.1.syms.defined
    }

    fn bodies(&self, o: &Self::Object, subject: &str, cand: &str, name: &str, syms: &[&Sym]) -> Result<Vec<Pin>> {
        pins_of(self.llvm, &o.1, subject, cand, name, syms, &RenameList::default())
    }

    fn done(&self, o: &Self::Object) -> Result<()> {
        o.0.check_intact()
    }
}

/// Leg (2) check against the committed pins, under an optional rename list (old → new) and
/// named moves. Returns the receipt; RED on a committed pin set that is not the frozen one (read
/// for every subject before anything is built), a disposition the built object contradicts
/// ([`contradicted`]), an absent pinned name, or an unnamed move.
///
/// Every subject the candidate list names is built, including one with no pin file (all of its
/// pairs recorded absent). `subjects` narrows the builds, and with them the object half: a
/// subject that is not built has its dispositions read but not re-verified.
pub fn check(ctx: &Ctx, llvm: &Llvm, rename: &RenameList, named: &[(String, String)], subjects: &[Subject]) -> Result<String> {
    let out = crate::probes::header(ctx, llvm, "leg (2) check");
    check_with(&ctx.root, &BuiltObjects { ctx, llvm }, rename, named, subjects, out)
}

/// [`check`]'s whole decision over the committed data under `root`, with the objects supplied by
/// `objects`; `out` is the receipt so far, and the receipt is returned (or carried by the RED).
///
/// In order: every subject's pin file is read and proven the frozen one ([`read_pins`]), before
/// any object is asked for; each subject of `subjects` that the candidate list names is then
/// taken from `objects` — with no pin file too, when every pair of it is recorded absent — and
/// its dispositions ([`contradicted`]), pinned names and bodies are checked against that object.
/// The verdict is RED [`RedKind::EmptyCensus`] when no pin was checked, else RED
/// [`RedKind::PinSet`] on a contradiction, else RED [`RedKind::SymbolAbsent`] on an absent pinned
/// name, else RED [`RedKind::Mismatch`] on an unnamed move.
pub fn check_with<O: Objects>(
    root: &Path,
    objects: &O,
    rename: &RenameList,
    named: &[(String, String)],
    subjects: &[Subject],
    mut out: String,
) -> Result<String> {
    let _ = writeln!(out, "rename entries {}, named moves {}", rename.0.len(), named.len());
    // Every subject's file is verified against the frozen list, not only the ones built below:
    // the pin set is one committed dataset, and `--subjects` narrows the builds, not the data.
    let mut files: Vec<(Subject, PinFile)> = Vec::with_capacity(subjects.len());
    let mut verified = 0usize;
    for s in SUBJECTS {
        let file = match read_pins(root, s.key)? {
            Some(file) => {
                verified += 1;
                file
            }
            // No frozen row and no file: every pair of the subject is recorded absent, and those
            // claims are still re-verified against its object below.
            None if CANDIDATES.iter().any(|c| c.subjects.contains(&s.key)) => PinFile::default(),
            None => continue,
        };
        if subjects.contains(&s) {
            files.push((s, file));
        }
    }
    let recorded_absent = read_absent(root)?;
    let _ = writeln!(
        out,
        "pin set: {} frozen pin(s); {verified} pin file(s) hold exactly their frozen rows; every candidate is frozen or recorded absent",
        read_frozen(root)?.len()
    );
    let mut absent: Vec<String> = Vec::new();
    let mut moved: Vec<String> = Vec::new();
    let mut named_moved: Vec<String> = Vec::new();
    let mut contradictions: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut reverified_absent = 0usize;
    for (s, file) in files {
        let o = objects.object(s)?;
        let defined = objects.defined(&o);
        reverified_absent += recorded_absent.iter().filter(|a| a.subject == s.key).count();
        for f in contradicted(s.key, defined, &file, &recorded_absent, rename) {
            let _ = writeln!(out, "CONTRADICTED {f}");
            contradictions.push(f);
        }
        let mut by_name_pins: BTreeMap<String, Vec<Pin>> = BTreeMap::new();
        for p in &file.pins {
            by_name_pins.entry(p.name.clone()).or_default().push(p.clone());
        }
        for (old_name, old_pins) in &by_name_pins {
            checked += old_pins.len();
            let cand = old_pins[0].cand.clone();
            let name = rename.apply(old_name);
            let syms = by_name_in(defined, &name);
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
            let fresh = objects.bodies(&o, s.key, &cand, &name, &syms)?;
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
        objects.done(&o)?;
    }
    let _ = writeln!(out, "checked {checked} pin(s): {} absent, {} moved unnamed, {} moved named", absent.len(), moved.len(), named_moved.len());
    let _ = writeln!(out, "dispositions re-verified against the objects: {reverified_absent} recorded-absent pair(s); {} contradiction(s)", contradictions.len());
    if checked == 0 {
        return Err(Red::new(RedKind::EmptyCensus, format!("{out}leg (2) found no committed pin to check")));
    }
    if !contradictions.is_empty() {
        return Err(Red::new(RedKind::PinSet, format!("{out}leg (2) RED: the built object(s) contradict the committed dispositions: {contradictions:?}")));
    }
    if !absent.is_empty() {
        return Err(Red::new(RedKind::SymbolAbsent, format!("{out}leg (2) RED: pinned symbol(s) absent: {absent:?}")));
    }
    if !moved.is_empty() {
        return Err(Red::new(RedKind::Mismatch, format!("{out}leg (2) RED: {} unnamed move(s)", moved.len())));
    }
    Ok(out)
}
