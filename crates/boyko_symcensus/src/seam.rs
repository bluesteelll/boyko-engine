//! Leg (5), the seam inventory and S-1 shape (03 §6; 05 §3, §6), and the name patterns leg (7b)
//! matches against rlib symbols.
//!
//! [`SEAM_INVENTORY`] mirrors 05 §3.2 for the items whose rung is on the trunk. At B3 that is
//! MS-08's four by-id operations, which are marked by doc lines only (05 §3.2: "doc lines only",
//! no code). The rung that lands a seam item adds its row here in the same commit
//! (D-S1(ii) adds MS-03's four readers).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::red::{Red, RedKind, Result};
use crate::source::{self, CENSUS_CRATES, Marker, SeamFacts, Stats, Via};

/// One seam item of 05 §3.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeamRow {
    /// The MS id (`MS-08`).
    pub ms: &'static str,
    /// The file that defines the item.
    pub file: &'static str,
    /// The item path as the leg-(1) walker spells it (`EcsMaster::add_component_by_id`).
    pub item: &'static str,
    /// MS-08's shape: a marker on an engine item that adds no code, exempt from the S-1 shape
    /// check (05 §6) and from leg (7b) (cut PC-6).
    pub doc_only: bool,
    /// The fragment of a demangled symbol path that names the item (leg 7b).
    pub symbol: &'static str,
}

/// The seam items on the trunk (05 §3.2), keyed by (item, MS id).
pub const SEAM_INVENTORY: &[SeamRow] = &[
    SeamRow {
        ms: "MS-08",
        file: "crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs",
        item: "EcsMaster::add_component_by_id",
        doc_only: true,
        symbol: "EcsMaster>::add_component_by_id",
    },
    SeamRow {
        ms: "MS-08",
        file: "crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs",
        item: "EcsMaster::remove_component_by_id",
        doc_only: true,
        symbol: "EcsMaster>::remove_component_by_id",
    },
    SeamRow {
        ms: "MS-08",
        file: "crates/boyko_ecs/src/ecs/core/ecs_master/seam_by_id.rs",
        item: "EcsMaster::mark_component_changed",
        doc_only: true,
        symbol: "EcsMaster>::mark_component_changed",
    },
    SeamRow {
        ms: "MS-08",
        file: "crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs",
        item: "EnableTagId::try_from_component_id",
        doc_only: true,
        symbol: "EnableTagId>::try_from_component_id",
    },
];

/// The file floor leg (5) must walk (the six crates held 326 `.rs` files at B3).
pub const MIN_FILES: usize = 250;

/// What leg (5) read.
#[derive(Clone, Debug, Default)]
pub struct SeamCensus {
    /// Files and items walked.
    pub stats: Stats,
    /// Every seam fact across the six crates.
    pub facts: SeamFacts,
}

/// Walks the six census crates for leg (5)'s facts.
pub fn census(root: &Path) -> Result<SeamCensus> {
    let mut out = SeamCensus::default();
    for c in CENSUS_CRATES {
        let src = root.join("crates").join(c).join("src");
        for f in source::rs_files(&src)? {
            let parsed = source::parse_path(&f)?;
            let (_, st, facts) = source::walk_file(&source::rel(root, &f), &parsed, Via::SynItem);
            out.stats.files += st.files;
            out.stats.items += st.items;
            out.facts.markers.extend(facts.markers);
            out.facts.impls_outside_test.extend(facts.impls_outside_test);
            out.facts.root_reexports.extend(facts.root_reexports);
            out.facts.root_globs.extend(facts.root_globs);
            out.facts.trait_defs.extend(facts.trait_defs);
        }
        out.stats.crates += 1;
    }
    out.facts.markers.sort();
    Ok(out)
}

/// Checks leg (5) on `c` and returns its receipt; RED on any violation.
///
/// 1. The marked set equals [`SEAM_INVENTORY`] exactly, keyed by (item, MS id).
/// 2. S-1 shape: every marked item that is not doc-only is generic over a `ModSeam`-bounded
///    parameter.
/// 3. No `impl ModSeam` outside `#[cfg(test)]`; no crate root re-exports `ModSeam`.
pub fn check(c: &SeamCensus) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "leg (5) read: {} crate(s), {} file(s), {} item(s)", c.stats.crates, c.stats.files, c.stats.items);
    if c.stats.files < MIN_FILES {
        return Err(Red::new(RedKind::EmptyCensus, format!("leg (5) walked {} files, under the floor of {MIN_FILES}", c.stats.files)));
    }
    let found: BTreeSet<(String, String)> = c.facts.markers.iter().map(|m| (m.item.clone(), m.ms.clone())).collect();
    let want: BTreeSet<(String, String)> = SEAM_INVENTORY.iter().map(|r| (r.item.to_owned(), r.ms.to_owned())).collect();
    let _ = writeln!(out, "markers found: {} (inventory: {})", c.facts.markers.len(), SEAM_INVENTORY.len());
    for m in &c.facts.markers {
        let _ = writeln!(out, "  {} | {} | {} | generic over ModSeam: {}", m.ms, m.item, m.file, m.generic_over_seam);
    }
    let mut problems = Vec::new();
    for (item, ms) in found.difference(&want) {
        problems.push(format!("marked but not in SEAM_INVENTORY: {ms} {item}"));
    }
    for (item, ms) in want.difference(&found) {
        problems.push(format!("in SEAM_INVENTORY but not marked: {ms} {item}"));
    }
    for m in &c.facts.markers {
        if !m.ms.starts_with("MS-") {
            problems.push(format!("a MOD-SEAM line names no MS id: {} in {} (`{}`)", m.item, m.file, m.ms));
        }
        if let Some(row) = SEAM_INVENTORY.iter().find(|r| r.item == m.item && r.ms == m.ms)
            && row.file != m.file
        {
            problems.push(format!("{} {} is marked in {}, the inventory says {}", m.ms, m.item, m.file, row.file));
        }
    }
    let code_adding: Vec<&Marker> =
        c.facts.markers.iter().filter(|m| !SEAM_INVENTORY.iter().any(|r| r.item == m.item && r.ms == m.ms && r.doc_only)).collect();
    let _ = writeln!(
        out,
        "S-1 shape: {} code-adding item(s) ({})",
        code_adding.len(),
        if c.facts.trait_defs.is_empty() { "ModSeam absent".to_owned() } else { format!("ModSeam defined in {:?}", c.facts.trait_defs) }
    );
    for m in &code_adding {
        if !m.generic_over_seam {
            problems.push(format!("S-1: {} {} adds code and is not generic over a ModSeam-bounded parameter", m.ms, m.item));
        }
    }
    let _ = writeln!(out, "impl ModSeam outside #[cfg(test)]: {} ; ModSeam re-exported at a crate root: {} ; root globs (unresolvable, listed): {}",
        c.facts.impls_outside_test.len(), c.facts.root_reexports.len(), c.facts.root_globs.len());
    for (f, i) in &c.facts.impls_outside_test {
        problems.push(format!("{i} outside #[cfg(test)] in {f}"));
    }
    for (f, u) in &c.facts.root_reexports {
        problems.push(format!("`pub use {u}` re-exports ModSeam at the crate root {f}"));
    }
    for (f, u) in &c.facts.root_globs {
        let _ = writeln!(out, "  root glob: pub use {u} ({f})");
    }
    if problems.is_empty() {
        let _ = writeln!(out, "leg (5): GREEN");
        Ok(out)
    } else {
        Err(Red::new(RedKind::Mismatch, format!("{out}leg (5) RED:\n  {}", problems.join("\n  "))))
    }
}

/// `true` if `demangled` names `row`'s item: the row's symbol fragment, not followed by an
/// identifier character (so `…::add_component_by_id_raw` does not match).
#[must_use]
pub fn names_item(demangled: &str, row: &SeamRow) -> bool {
    let mut rest = demangled;
    while let Some(i) = rest.find(row.symbol) {
        let after = rest[i + row.symbol.len()..].chars().next();
        if !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            return true;
        }
        rest = &rest[i + row.symbol.len()..];
    }
    false
}
