//! Leg (7), the linked seam census (03 §6): a snapshot per subject and a strict comparison.
//!
//! (a) `llvm-size -A` of the IMAGE; (b) the post-LTO OBJECT's defined multiset
//! `(normalised name, class, size)`; (c) the image's export directory. `link.exe` writes no symbol
//! table into the image, so (b) is read from the object; `/OPT:REF` can only remove, so an object
//! that lacks a symbol implies an image that lacks it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::llvm;
use crate::normalize::RenameList;
use crate::objbuild::Built;
use crate::objview::{Llvm, ObjView};
use crate::red::{Red, RedKind, Result};

/// The first line of every snapshot file.
pub const MAGIC: &str = "# ug15 leg7 snapshot v1";

/// One leg-(7) snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// `# …` provenance lines (tools, host, subject, profile, digests). Not compared.
    pub header: Vec<String>,
    /// (a) `(section, size)`, including `Total`.
    pub sections: Vec<(String, u64)>,
    /// (b) `(name, class, size) → count`.
    pub multiset: BTreeMap<(String, char, u64), usize>,
    /// (c) export lines.
    pub exports: Vec<String>,
}

impl Snapshot {
    /// Captures (a)–(c) from a paired build; RED on an empty census anywhere.
    pub fn capture(tools: &Llvm, built: &Built, header: Vec<String>) -> Result<Self> {
        let view = ObjView::load(tools, &built.object)?;
        let multiset = view.multiset(&RenameList::default())?;
        let sections = llvm::size_a(&tools.size, &built.image)?;
        let exports = llvm::coff_exports(&tools.readobj, &built.image)?;
        built.check_intact()?;
        Ok(Self { header, sections, multiset, exports })
    }

    /// The file text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(MAGIC);
        s.push('\n');
        for h in &self.header {
            let _ = writeln!(s, "# {h}");
        }
        s.push_str("[a] llvm-size -A (image)\n");
        for (n, v) in &self.sections {
            let _ = writeln!(s, "{n}\t{v}");
        }
        let total: usize = self.multiset.values().sum();
        let _ = writeln!(s, "[b] defined multiset (object): {} distinct, {total} symbols; count\\tclass\\tsize\\tname", self.multiset.len());
        for ((name, class, size), count) in &self.multiset {
            let _ = writeln!(s, "{count}\t{class}\t{size}\t{name}");
        }
        let _ = writeln!(s, "[c] coff exports (image): {}", self.exports.len());
        for e in &self.exports {
            let _ = writeln!(s, "{e}");
        }
        s
    }

    /// Parses a snapshot file.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err(Red::new(RedKind::Malformed, format!("not a snapshot: the first line is not `{MAGIC}`")));
        }
        let mut snap = Self::default();
        let mut part = ' ';
        for line in lines {
            if let Some(h) = line.strip_prefix("# ") {
                snap.header.push(h.to_owned());
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                part = rest.chars().next().unwrap_or(' ');
                continue;
            }
            match part {
                'a' => {
                    let (n, v) = line.split_once('\t').ok_or_else(|| bad(line))?;
                    snap.sections.push((n.to_owned(), v.parse().map_err(|_| bad(line))?));
                }
                'b' => {
                    let mut it = line.splitn(4, '\t');
                    let (Some(c), Some(k), Some(z), Some(n)) = (it.next(), it.next(), it.next(), it.next()) else {
                        return Err(bad(line));
                    };
                    let class = k.chars().next().ok_or_else(|| bad(line))?;
                    snap.multiset.insert(
                        (n.to_owned(), class, z.parse().map_err(|_| bad(line))?),
                        c.parse().map_err(|_| bad(line))?,
                    );
                }
                'c' => snap.exports.push(line.to_owned()),
                _ => return Err(bad(line)),
            }
        }
        if snap.sections.is_empty() || snap.multiset.is_empty() {
            return Err(Red::new(RedKind::EmptyCensus, "a snapshot with no section row or no symbol is not a baseline"));
        }
        Ok(snap)
    }
}

#[cold]
fn bad(line: &str) -> Red {
    Red::new(RedKind::Malformed, format!("snapshot line not understood: {line}"))
}

/// The differences between `parent` (names mapped through `rename`) and `child`; empty = pass.
#[must_use]
pub fn compare(parent: &Snapshot, child: &Snapshot, rename: &RenameList) -> Vec<String> {
    let mut diffs = Vec::new();
    if parent.sections != child.sections {
        diffs.push(format!("(a) sections: parent {:?} / child {:?}", parent.sections, child.sections));
    }
    let mut p: BTreeMap<(String, char, u64), isize> = BTreeMap::new();
    for ((n, c, z), k) in &parent.multiset {
        *p.entry((rename.apply(n), *c, *z)).or_insert(0) += isize::try_from(*k).unwrap_or(isize::MAX);
    }
    for (key, k) in &child.multiset {
        *p.entry(key.clone()).or_insert(0) -= isize::try_from(*k).unwrap_or(isize::MAX);
    }
    for ((n, c, z), d) in p {
        if d != 0 {
            let side = if d > 0 { "parent only" } else { "child only" };
            diffs.push(format!("(b) {side} ×{}: {c} {z} {n}", d.unsigned_abs()));
        }
    }
    if parent.exports != child.exports {
        diffs.push(format!("(c) exports: parent {:?} / child {:?}", parent.exports, child.exports));
    }
    diffs
}
