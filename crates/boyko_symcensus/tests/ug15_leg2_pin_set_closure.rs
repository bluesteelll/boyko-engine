//! Leg (2)'s pin-set closure (B3 review W1, retest R1/R2/R5): every single edit of the committed
//! pin data, run through the readers every leg uses, on scratch copies.
//!
//! Ported from the B3 retest's out-of-tree sweep. The committed data of THIS tree is only read; each
//! case is written under `CARGO_TARGET_TMPDIR` and read back from there.
//!
//! Two halves of the gate are replicated, each exactly as `pins::check` runs it:
//! - **the read** — `pins::read_pins` for every subject of `SUBJECTS`, before any build, RED on the
//!   first error;
//! - **the object** — `pins::contradicted` per subject, against a MODEL object whose defined code
//!   symbols are the names the committed frozen list pins for that subject (what capture found),
//!   plus whatever a case adds. It stands in for the post-LTO object the binary builds; the
//!   binary's own end-to-end runs are the B3 receipts.
//!
//! An edit that is consistent across the three files (family C: one name of a multi-name candidate
//! cut from the pin file and the frozen list; family E: a pinned pair moved into the absent list)
//! reads GREEN by construction — the data is self-consistent — and is RED at the object, which
//! still defines the names the data no longer claims. Both halves are asserted, so a read that
//! starts REDding them and an object check that stops are both failures here.
//!
//! What lies after the object check in the binary (the body comparison, MOVED / ABSENT, the
//! `checked == 0` guard) is not replicated: family M3 and the vacuity probe Q record where that
//! part takes over.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use boyko_symcensus::candidates::{CANDIDATES, Rule};
use boyko_symcensus::llvm::Sym;
use boyko_symcensus::normalize::{RenameList, norm_name};
use boyko_symcensus::objbuild::SUBJECTS;
use boyko_symcensus::pins::{self, PinFile};
use boyko_symcensus::red::RedKind;
use boyko_symcensus::sha256;

/// (subject, cand, name, sha256) of one frozen row or pin block.
type Key = (String, String, String, String);

/// The committed frozen list's row count and the absent list's pair count on this tree. A
/// re-capture that changes either must re-read this sweep's expectations, so they are pinned.
const FROZEN_ROWS: usize = 42;
const ABSENT_PAIRS: usize = 6;

/// The tree whose committed data is swept (read only).
fn tree_root() -> PathBuf {
    let m = Path::new(env!("CARGO_MANIFEST_DIR"));
    m.parent().and_then(Path::parent).expect("test setup: the crate sits two levels below the workspace root").to_path_buf()
}

#[derive(Clone)]
struct Data {
    pins: Vec<(&'static str, Option<String>)>,
    frozen: Option<String>,
    absent: Option<String>,
}

impl Data {
    fn load(root: &Path) -> Self {
        let pins = SUBJECTS.iter().map(|s| (s.key, fs::read_to_string(pins::pin_path(root, s.key)).ok())).collect();
        Self {
            pins,
            frozen: Some(fs::read_to_string(pins::frozen_path(root)).expect("test setup: the committed frozen list")),
            absent: Some(fs::read_to_string(pins::absent_path(root)).expect("test setup: the committed absent list")),
        }
    }

    fn pin(&mut self, subject: &str) -> &mut Option<String> {
        &mut self.pins.iter_mut().find(|(s, _)| *s == subject).expect("test setup: a known subject").1
    }

    /// Writes the case under `root`: a file that is `None` here is removed there, so every case
    /// sees exactly its own files.
    fn write(&self, root: &Path) {
        let dir = pins::data_dir(root).join("pins");
        fs::create_dir_all(&dir).expect("test setup: case dir");
        let mut files: Vec<(PathBuf, Option<&String>)> = self.pins.iter().map(|(s, t)| (pins::pin_path(root, s), t.as_ref())).collect();
        files.push((pins::frozen_path(root), self.frozen.as_ref()));
        files.push((pins::absent_path(root), self.absent.as_ref()));
        for (p, t) in files {
            match t {
                Some(t) => fs::write(&p, t).expect("test setup: write a case file"),
                None if p.exists() => fs::remove_file(&p).expect("test setup: remove a case file"),
                None => {}
            }
        }
    }
}

fn eol(t: &str) -> &'static str {
    if t.contains("\r\n") { "\r\n" } else { "\n" }
}

fn split_blocks(text: &str) -> (String, Vec<String>) {
    let mut pre = String::new();
    let mut blocks: Vec<String> = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("== ") {
            blocks.push(line.to_owned());
        } else if let Some(b) = blocks.last_mut() {
            b.push_str(line);
        } else {
            pre.push_str(line);
        }
    }
    (pre, blocks)
}

fn header_line(block: &str) -> &str {
    block.split_inclusive('\n').next().expect("a block has a header")
}

fn header_key(block: &str) -> Key {
    let h = header_line(block).trim_end_matches(['\r', '\n']);
    let p: Vec<&str> = h.strip_prefix("== ").expect("a pin header").split(" | ").collect();
    assert_eq!(p.len(), 6, "a pin header has six fields: {h}");
    (p[0].to_owned(), p[1].to_owned(), p[2].to_owned(), p[5].strip_prefix("sha256=").expect("the sha field").to_owned())
}

fn is_row(l: &str) -> bool {
    !l.starts_with('#') && !l.trim().is_empty()
}

fn row_fields(l: &str) -> Vec<String> {
    l.trim_end_matches(['\r', '\n']).split('\t').map(str::to_owned).collect()
}

fn row_key(l: &str) -> Key {
    let f = row_fields(l);
    (f[0].clone(), f[1].clone(), f[2].clone(), f[5].clone())
}

fn frozen_rows(d: &Data) -> Vec<Key> {
    d.frozen.as_deref().unwrap_or("").split_inclusive('\n').filter(|l| is_row(l)).map(row_key).collect()
}

fn absent_rows(d: &Data) -> Vec<(String, String)> {
    d.absent
        .as_deref()
        .unwrap_or("")
        .split_inclusive('\n')
        .filter(|l| is_row(l))
        .map(|l| {
            let f: Vec<&str> = l.splitn(3, '\t').collect();
            (f[0].to_owned(), f[1].to_owned())
        })
        .collect()
}

/// Removes the first block of `k.0`'s pin file whose header key is `k`, and returns it.
fn remove_block(d: &mut Data, k: &Key) -> String {
    let t = d.pin(&k.0).as_ref().expect("the pin file is present").clone();
    let (pre, mut bl) = split_blocks(&t);
    let i = bl.iter().position(|b| header_key(b) == *k).expect("the frozen row's block exists");
    let removed = bl.remove(i);
    *d.pin(&k.0) = Some(pre + &bl.concat());
    removed
}

/// Edits the first block with key `k` through `f(header, body) -> (header, body)`.
fn edit_block(d: &mut Data, k: &Key, f: impl FnOnce(String, String) -> (String, String)) {
    let t = d.pin(&k.0).as_ref().expect("the pin file is present").clone();
    let (pre, mut bl) = split_blocks(&t);
    let i = bl.iter().position(|b| header_key(b) == *k).expect("the block exists");
    let h = header_line(&bl[i]).to_owned();
    let body = bl[i][h.len()..].to_owned();
    let (h2, b2) = f(h, body);
    bl[i] = h2 + &b2;
    *d.pin(&k.0) = Some(pre + &bl.concat());
}

/// Edits (or removes, when `f` returns `None`) the first frozen row with key `k`.
fn edit_row(d: &mut Data, k: &Key, f: impl FnOnce(Vec<String>) -> Option<Vec<String>>) {
    let t = d.frozen.as_ref().expect("the frozen list is present").clone();
    let e = eol(&t);
    let mut out = String::new();
    let mut f = Some(f);
    for l in t.split_inclusive('\n') {
        if f.is_some() && is_row(l) && row_key(l) == *k {
            if let Some(r) = (f.take().expect("taken once"))(row_fields(l)) {
                out.push_str(&r.join("\t"));
                out.push_str(e);
            }
        } else {
            out.push_str(l);
        }
    }
    assert!(f.is_none(), "frozen row {k:?} found");
    d.frozen = Some(out);
}

fn append(t: &mut Option<String>, line: &str) {
    let s = t.get_or_insert_with(String::new);
    let e = eol(s);
    if !s.is_empty() && !s.ends_with('\n') {
        s.push_str(e);
    }
    s.push_str(line);
    s.push_str(e);
}

fn body_sha(body: &str) -> String {
    sha256::bytes_hex(body.replace("\r\n", "\n").as_bytes())
}

/// A defined code symbol of the model object.
fn code(i: usize, name: &str) -> Sym {
    Sym { raw: format!("_RNvCs_ug15_model_{i}"), name: name.to_owned(), class: 't', value: Some(0) }
}

/// The model object of every subject: the names the committed frozen list pins for it.
fn model_objects(rows: &[Key]) -> Vec<(&'static str, Vec<Sym>)> {
    SUBJECTS
        .iter()
        .map(|s| {
            let mut names: Vec<&str> = rows.iter().filter(|k| k.0 == s.key).map(|k| k.2.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            (s.key, names.iter().enumerate().map(|(i, n)| code(i, n)).collect())
        })
        .collect()
}

/// A name the first rule of candidate `cand` locates, for adding to a model object.
fn a_name_located_by(cand: &str) -> String {
    let c = CANDIDATES.iter().find(|c| c.id == cand).expect("test setup: a known candidate");
    match c.rules[0] {
        Rule::Exact(n) => n.to_owned(),
        Rule::Contains(sub) => format!("ug15_model::{sub}::{{closure#0}}"),
        Rule::FirstWithPrefix(p) => format!("{p}ug15_model::Reappeared>"),
    }
}

#[derive(Debug)]
enum Out {
    Green(usize),
    Red(RedKind, String),
}

impl Out {
    fn show(&self) -> String {
        match self {
            Out::Green(m) => format!("GREEN {m} pin(s)"),
            Out::Red(k, det) => format!("RED [{}] {}", k.label(), det.chars().take(160).collect::<String>()),
        }
    }
}

/// The read half: `read_pins` for every subject, as `check` runs it before any build.
fn read_gate(root: &Path) -> Out {
    let mut n = 0;
    for s in SUBJECTS {
        match pins::read_pins(root, s.key) {
            Ok(Some(f)) => n += f.pins.len(),
            Ok(None) => {}
            Err(r) => return Out::Red(r.kind, r.detail),
        }
    }
    Out::Green(n)
}

/// The object half: `contradicted` per subject against its model object, as `check` runs it after
/// each build (every subject the candidate list names; `PinFile::default()` for one with no file).
fn object_gate(root: &Path, objects: &[(&'static str, Vec<Sym>)]) -> Out {
    let absent = match pins::read_absent(root) {
        Ok(a) => a,
        Err(r) => return Out::Red(r.kind, r.detail),
    };
    let mut faults: Vec<String> = Vec::new();
    for (subject, defined) in objects {
        if !CANDIDATES.iter().any(|c| c.subjects.contains(subject)) {
            continue;
        }
        let file = match pins::read_pins(root, subject) {
            Ok(f) => f.unwrap_or_else(PinFile::default),
            Err(r) => return Out::Red(r.kind, r.detail),
        };
        faults.extend(pins::contradicted(subject, defined, &file, &absent, &RenameList::default()));
    }
    if faults.is_empty() { Out::Green(0) } else { Out::Red(RedKind::PinSet, format!("{faults:?}")) }
}

#[derive(Clone, Debug)]
enum Expect {
    /// Both halves green; the read counts this many pins.
    Green(usize),
    /// The read REDs with this kind (the object half never runs: `check` stops first).
    Red(RedKind),
    /// The read REDs with this kind, and its sentence names this text (R2: the diagnostic names
    /// the row it is about).
    RedNaming(RedKind, String),
    /// The read is green with this many pins, and the object REDs [pin set] naming this text.
    ObjectRed(usize, &'static str),
}

struct Runner {
    root: PathBuf,
    table: String,
    failures: Vec<String>,
    /// (family, cases, red at the read, red at the object)
    per_family: Vec<(String, usize, usize, usize)>,
}

impl Runner {
    fn run(&mut self, family: &str, name: &str, d: &Data, objects: &[(&'static str, Vec<Sym>)], want: &Expect) {
        d.write(&self.root);
        let read = read_gate(&self.root);
        let object = match read {
            Out::Green(_) => Some(object_gate(&self.root, objects)),
            Out::Red(..) => None,
        };
        let ok = match (want, &read, &object) {
            (Expect::Green(n), Out::Green(m), Some(Out::Green(_))) => n == m,
            (Expect::Red(k), Out::Red(j, _), None) => k == j,
            (Expect::RedNaming(k, needle), Out::Red(j, det), None) => k == j && det.contains(needle.as_str()),
            (Expect::ObjectRed(n, needle), Out::Green(m), Some(Out::Red(RedKind::PinSet, det))) => n == m && det.contains(needle),
            _ => false,
        };
        let got = format!("read {}; object {}", read.show(), object.as_ref().map_or_else(|| "not reached".to_owned(), Out::show));
        let want_s = format!("{want:?}");
        let _ = writeln!(self.table, "{} {family:<3} {name:<80} want {want_s:<40} got {got}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failures.push(format!("{family} {name}: want {want:?}, got {got}"));
        }
        let i = match self.per_family.iter().position(|(f, ..)| f == family) {
            Some(i) => i,
            None => {
                self.per_family.push((family.to_owned(), 0, 0, 0));
                self.per_family.len() - 1
            }
        };
        let e = &mut self.per_family[i];
        e.1 += 1;
        e.2 += usize::from(matches!(read, Out::Red(..)));
        e.3 += usize::from(matches!(object, Some(Out::Red(..))));
    }
}

fn short(k: &Key) -> String {
    format!("{}/{} {} {}", k.0, k.1, k.2.chars().take(40).collect::<String>(), &k.3[..10])
}

#[test]
fn every_single_edit_of_the_committed_pin_data() {
    let base = Data::load(&tree_root());
    let rows = frozen_rows(&base);
    let absent = absent_rows(&base);
    assert_eq!(rows.len(), FROZEN_ROWS, "the committed frozen list holds the pins capture wrote");
    assert_eq!(absent.len(), ABSENT_PAIRS, "the committed absent list's pair count");
    for k in &rows {
        assert_eq!(norm_name(&k.2), k.2, "a frozen name is already normalised, so the model object locates it as capture did");
    }
    let pair_count = |c: &str, s: &str| rows.iter().filter(|k| k.1 == c && k.0 == s).count();
    let mut pairs: Vec<(String, String)> = rows.iter().map(|k| (k.1.clone(), k.0.clone())).collect();
    pairs.sort();
    pairs.dedup();
    let n_pairs: usize = CANDIDATES.iter().map(|c| c.subjects.len()).sum();
    assert_eq!(pairs.len() + absent.len(), n_pairs, "frozen pairs + absent pairs = every candidate x subject pair");
    let objects = model_objects(&rows);

    let mut r = Runner {
        root: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ug15_leg2_pin_set_closure").join("case"),
        table: String::new(),
        failures: Vec::new(),
        per_family: Vec::new(),
    };

    // 0. The committed data against the object capture read it from.
    r.run("0", "baseline: the committed data", &base, &objects, &Expect::Green(FROZEN_ROWS));

    for k in &rows {
        // A. One pinned block cut from its pin file only.
        let mut d = base.clone();
        remove_block(&mut d, k);
        r.run("A", &format!("block cut from pins only: {}", short(k)), &d, &objects, &Expect::Red(RedKind::PinSet));
        // B. One frozen row removed only.
        let mut d = base.clone();
        edit_row(&mut d, k, |_| None);
        r.run("B", &format!("row cut from frozen only: {}", short(k)), &d, &objects, &Expect::Red(RedKind::PinSet));
        // C. Cut from both, consistently: the read REDs only when it was the pair's last row; a
        // multi-name candidate's cut name is still in the object, which its rule locates.
        let mut d = base.clone();
        remove_block(&mut d, k);
        edit_row(&mut d, k, |_| None);
        let want = if pair_count(&k.1, &k.0) == 1 { Expect::Red(RedKind::PinSet) } else { Expect::ObjectRed(FROZEN_ROWS - 1, "located, but not frozen") };
        r.run("C", &format!("cut from pins AND frozen: {} (pair rows {})", short(k), pair_count(&k.1, &k.0)), &d, &objects, &want);
        // H. The frozen row disagrees with the block on one field.
        for (field, what) in [(5usize, "sha"), (3, "size"), (4, "copies"), (2, "name")] {
            let mut d = base.clone();
            edit_row(&mut d, k, |mut f| {
                f[field] = match field {
                    5 => format!("{}{}", if f[5].starts_with('0') { '1' } else { '0' }, &f[5][1..]),
                    3 | 4 => (f[field].parse::<u64>().expect("a number") + 1).to_string(),
                    _ => format!("{}x", f[2]),
                };
                Some(f)
            });
            r.run("H", &format!("frozen {what} differs: {}", short(k)), &d, &objects, &Expect::Red(RedKind::PinSet));
        }
        // I. A block duplicated in its pin file: a count change with no new row, which the
        // diagnostic must still name (R2).
        let mut d = base.clone();
        edit_block(&mut d, k, |h, b| (h.clone(), format!("{b}{h}{b}")));
        r.run("I", &format!("block duplicated: {}", short(k)), &d, &objects, &Expect::RedNaming(RedKind::PinSet, format!("{} `{}` {}", k.1, k.2, &k.3[..12])));
        // L. A block moved into another subject's pin file.
        let to = SUBJECTS.iter().map(|s| s.key).find(|s| *s != k.0).expect("another subject");
        let mut d = base.clone();
        let blk = remove_block(&mut d, k);
        let t = d.pin(to).as_ref().expect("the pin file").clone();
        *d.pin(to) = Some(t + &blk);
        r.run("L", &format!("block moved to {to}'s file: {}", short(k)), &d, &objects, &Expect::Red(RedKind::PinSet));
        // M1. Body edited by hand, header untouched.
        let mut d = base.clone();
        edit_block(&mut d, k, |h, b| (h, format!("{b}; ug15 sweep\n")));
        r.run("M1", &format!("body hand-edited: {}", short(k)), &d, &objects, &Expect::Red(RedKind::Malformed));
        // M2. Body and its header sha edited, frozen untouched.
        let mut d = base.clone();
        let mut new_sha = String::new();
        edit_block(&mut d, k, |h, b| {
            let nb = format!("{b}; ug15 sweep\n");
            new_sha = body_sha(&nb);
            (h.replace(&format!("sha256={}", k.3), &format!("sha256={new_sha}")), nb)
        });
        r.run("M2", &format!("body + header sha edited: {}", short(k)), &d, &objects, &Expect::Red(RedKind::PinSet));
        // M3. Body, header sha and frozen sha edited together: green here by design; the build's
        // body comparison calls it MOVED.
        edit_row(&mut d, k, |mut f| {
            f[5] = new_sha;
            Some(f)
        });
        r.run("M3", &format!("body + header + frozen sha edited: {}", short(k)), &d, &objects, &Expect::Green(FROZEN_ROWS));
    }

    for (c, s) in &absent {
        // D. An absent row removed.
        let mut d = base.clone();
        let t = d.absent.clone().expect("the absent list");
        let mut done = false;
        let kept: String = t
            .split_inclusive('\n')
            .filter(|l| {
                if !done && is_row(l) && l.starts_with(&format!("{c}\t{s}\t")) {
                    done = true;
                    false
                } else {
                    true
                }
            })
            .collect();
        assert!(done, "absent row {c} {s} found");
        d.absent = Some(kept);
        r.run("D", &format!("absent row removed: {c}/{s}"), &d, &objects, &Expect::Red(RedKind::PinSet));
        // P. The object now defines a symbol the absent candidate's rule locates: a real codegen
        // change (a candidate fat LTO used to inline away) that the data still calls absent.
        let mut objs = objects.clone();
        let obj = &mut objs.iter_mut().find(|(k, _)| k == s).expect("the subject's model object").1;
        obj.push(code(obj.len(), &a_name_located_by(c)));
        r.run("P", &format!("recorded absent, now present in the object: {c}/{s}"), &base, &objs, &Expect::ObjectRed(FROZEN_ROWS, "recorded absent, but present"));
    }

    for (c, s) in &pairs {
        // K. A pinned pair also recorded absent.
        let mut d = base.clone();
        append(&mut d.absent, &format!("{c}\t{s}\tforged by the ug15 sweep"));
        r.run("K", &format!("pinned pair also recorded absent: {c}/{s}"), &d, &objects, &Expect::Red(RedKind::PinSet));
        // E. A pinned pair moved to the absent list consistently (pins + frozen + absent).
        let mut d = base.clone();
        let ks: Vec<Key> = rows.iter().filter(|k| k.1 == *c && k.0 == *s).cloned().collect();
        for k in &ks {
            remove_block(&mut d, k);
            edit_row(&mut d, k, |_| None);
        }
        append(&mut d.absent, &format!("{c}\t{s}\tforged by the ug15 sweep (a pinned pair moved to absent)"));
        r.run("E", &format!("pair moved to absent in all 3 files: {c}/{s} ({} row(s))", ks.len()), &d, &objects, &Expect::ObjectRed(FROZEN_ROWS - ks.len(), "recorded absent, but present"));
        // W. A pinned FirstWithPrefix pair whose rule would now pick another, earlier
        // instantiation: cut D5 checks the frozen name forever, so this is NOT a contradiction.
        let cand = CANDIDATES.iter().find(|x| x.id == c).expect("a known candidate");
        if let [Rule::FirstWithPrefix(p)] = cand.rules {
            let mut objs = objects.clone();
            let obj = &mut objs.iter_mut().find(|(k, _)| k == s).expect("the subject's model object").1;
            obj.push(code(obj.len(), &format!("{p}aaa_ug15_model::Earlier>")));
            r.run("W", &format!("an earlier {c} instantiation appears in {s} (cut D5: not a fault)"), &base, &objs, &Expect::Green(FROZEN_ROWS));
        }
    }

    for s in SUBJECTS {
        // F. A pin file deleted.
        let mut d = base.clone();
        *d.pin(s.key) = None;
        r.run("F", &format!("pin file deleted: {}", s.key), &d, &objects, &Expect::Red(RedKind::PinSet));
        // S. A pin file reduced to its magic line.
        let mut d = base.clone();
        *d.pin(s.key) = Some(format!("{}\n", pins::MAGIC));
        r.run("S", &format!("pin file emptied to its magic line: {}", s.key), &d, &objects, &Expect::Red(RedKind::PinSet));
        // T. A pin file with a wrong magic line.
        let mut d = base.clone();
        let t = d.pin(s.key).clone().expect("the pin file");
        *d.pin(s.key) = Some(t.replacen(pins::MAGIC, "# ug15 leg2 pins v0", 1));
        r.run("T", &format!("pin file magic changed: {}", s.key), &d, &objects, &Expect::Red(RedKind::Malformed));
        // E2. Every pinned pair of the subject moved to absent AND its pin file deleted: no file
        // and no frozen row, so the object half must still build and re-verify the subject.
        let mut d = base.clone();
        let ks: Vec<Key> = rows.iter().filter(|k| k.0 == s.key).cloned().collect();
        for k in &ks {
            edit_row(&mut d, k, |_| None);
        }
        *d.pin(s.key) = None;
        let mut moved: Vec<&str> = ks.iter().map(|k| k.1.as_str()).collect();
        moved.sort_unstable();
        moved.dedup();
        for c in &moved {
            append(&mut d.absent, &format!("{c}\t{}\tforged by the ug15 sweep (the whole subject moved to absent)", s.key));
        }
        r.run("E2", &format!("all of {} moved to absent, pin file deleted ({} row(s))", s.key, ks.len()), &d, &objects, &Expect::ObjectRed(FROZEN_ROWS - ks.len(), "recorded absent, but present"));
    }

    // G. The frozen or the absent list deleted.
    let mut d = base.clone();
    d.frozen = None;
    r.run("G", "frozen list deleted", &d, &objects, &Expect::Red(RedKind::PinSet));
    let mut d = base.clone();
    d.absent = None;
    r.run("G", "absent list deleted", &d, &objects, &Expect::Red(RedKind::PinSet));

    // J. Rows naming no candidate pair.
    let mut d = base.clone();
    append(&mut d.frozen, &format!("clear\tZ-9\tno::such\t1\t1\t{}", "0".repeat(64)));
    r.run("J", "frozen row naming an unknown candidate (Z-9/clear)", &d, &objects, &Expect::Red(RedKind::PinSet));
    let mut d = base.clone();
    append(&mut d.absent, "Z-9\tclear\tforged by the ug15 sweep");
    r.run("J", "absent row naming an unknown candidate (Z-9/clear)", &d, &objects, &Expect::Red(RedKind::PinSet));
    let mut d = base.clone();
    append(&mut d.absent, "R-1\tboyko_demo\tforged by the ug15 sweep (R-1 is a clear-only candidate)");
    r.run("J", "absent row naming a known candidate in a subject it does not list (R-1/boyko_demo)", &d, &objects, &Expect::Red(RedKind::PinSet));
    // J4. A known candidate pinned in a subject it does not list, block + frozen row consistent.
    let k = rows.iter().find(|k| k.1 == "R-1").expect("R-1 is frozen").clone();
    let mut d = base.clone();
    let blk = remove_block(&mut base.clone(), &k);
    let moved = blk.replacen("== clear | R-1 |", "== boyko_demo | R-1 |", 1);
    let t = d.pin("boyko_demo").clone().expect("demo's pin file");
    *d.pin("boyko_demo") = Some(t + &moved);
    let line = d.frozen.as_deref().expect("frozen").split_inclusive('\n').find(|l| is_row(l) && row_key(l) == k).expect("R-1's frozen row").to_owned();
    let mut f = row_fields(&line);
    f[0] = "boyko_demo".into();
    append(&mut d.frozen, &f.join("\t"));
    r.run("J", "R-1 additionally pinned in boyko_demo (block + frozen row, consistent)", &d, &objects, &Expect::Red(RedKind::PinSet));
    // J5. A malformed frozen row.
    let mut d = base.clone();
    append(&mut d.frozen, "clear\tR-1\tonly-five-fields\t1\t1");
    r.run("J", "frozen row with five fields", &d, &objects, &Expect::Red(RedKind::Malformed));

    // N. Line endings flipped everywhere: the digests are over LF text.
    let mut d = base.clone();
    for (_, t) in &mut d.pins {
        if let Some(t) = t {
            let lf = t.replace("\r\n", "\n");
            *t = if lf == *t { lf.replace('\n', "\r\n") } else { lf };
        }
    }
    for t in [&mut d.frozen, &mut d.absent] {
        let x = t.clone().expect("present");
        let lf = x.replace("\r\n", "\n");
        *t = Some(if lf == x { lf.replace('\n', "\r\n") } else { lf });
    }
    r.run("N", "every file's line endings flipped (CRLF <-> LF)", &d, &objects, &Expect::Green(FROZEN_ROWS));

    // O. Row and block order reversed: the row SET is compared.
    let mut d = base.clone();
    for (_, t) in &mut d.pins {
        if let Some(x) = t {
            let (pre, mut bl) = split_blocks(x);
            bl.reverse();
            *x = pre + &bl.concat();
        }
    }
    let ft = d.frozen.clone().expect("frozen");
    let (hdr, mut body): (Vec<&str>, Vec<&str>) = ft.split_inclusive('\n').partition(|l| !is_row(l));
    body.reverse();
    d.frozen = Some(hdr.concat() + &body.concat());
    r.run("O", "block order and frozen row order reversed", &d, &objects, &Expect::Green(FROZEN_ROWS));

    // Q. Vacuity: frozen header-only, all pin files gone, every pinned pair forged absent. The
    // read is green with 0 pins (the binary's `checked == 0` guard is the one that sees that);
    // the object half contradicts every forged claim.
    let mut d = base.clone();
    d.frozen = Some(ft.split_inclusive('\n').filter(|l| !is_row(l)).collect());
    for (_, t) in &mut d.pins {
        *t = None;
    }
    for (c, s) in &pairs {
        append(&mut d.absent, &format!("{c}\t{s}\tforged by the ug15 sweep (vacuity probe)"));
    }
    r.run("Q", "vacuity: frozen header-only, no pin file, every pair absent", &d, &objects, &Expect::ObjectRed(0, "recorded absent, but present"));
    // R. Frozen header-only and no pin file, absent list untouched.
    let mut d = base.clone();
    d.frozen = Some(ft.split_inclusive('\n').filter(|l| !is_row(l)).collect());
    for (_, t) in &mut d.pins {
        *t = None;
    }
    r.run("R", "frozen header-only and no pin file, absent untouched", &d, &objects, &Expect::Red(RedKind::PinSet));

    let mut summary = String::new();
    for (f, n, red, obj) in &r.per_family {
        let _ = writeln!(summary, "family {f:<3} cases {n:>3}  red at the read {red:>3}  red at the object {obj:>3}");
    }
    let total: usize = r.per_family.iter().map(|e| e.1).sum();
    let table = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ug15_leg2_pin_set_closure").join("table.txt");
    fs::write(&table, format!("{}\n{summary}total cases {total}, failures {}\n", r.table, r.failures.len())).expect("write the case table");
    println!("{summary}total cases {total}, failures {} (table: {})", r.failures.len(), table.display());
    for f in &r.failures {
        println!("FAILURE {f}");
    }
    assert!(r.failures.is_empty(), "{} case(s) did not match:\n{}", r.failures.len(), r.failures.join("\n"));
}
