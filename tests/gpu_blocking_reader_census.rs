//! **G2a, clause 2 (profiling rung 4).** The set of source files naming `vkGetQueryPoolResults`
//! equals a pinned list, so a new blocking GPU query reader **fails this gate by existing**.
//!
//! # Why a census of FILES and not a grep for `WAIT_BIT`
//!
//! Rev 2's version grepped the profiling module for `VK_QUERY_RESULT_WAIT_BIT`. The verb's body
//! has to live in `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs`, beside its siblings — a file
//! that scope structurally excludes. A mechanical check whose scope excludes the defect is the
//! failure shape this corpus names repeatedly, so the scope moved: every file that can call the
//! FFI entry point at all is enumerated, and the enumeration is the assertion.
//!
//! The other half of G2a is a `const _: () = assert!(...)` on `GPU_ZONE_QUERY_FLAGS` in that same
//! file. The two are complementary and neither subsumes the other: the const-assert makes the
//! **existing** reader's flag word unable to carry `WAIT_BIT` (a build failure, which is the only
//! showable red — a blocking read HANGS, and this repository has no kill-after-timeout pattern),
//! while this census makes a **new** reader in a **new** file fail without anybody having thought
//! to add a const-assert to it.
//!
//! # Why this lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree — `internal_docs_anchors.rs`'s rationale, verbatim.
//!
//! # What it cannot claim
//!
//! It cannot claim the *driver* never blocks internally, only that this code never asks it to.
//! And it cannot claim a pinned file's reader is non-blocking — that is the const-assert's job for
//! the new seam, and the three FROZEN readers in `boyko_rhi::device` are blocking **by contract**
//! and are pinned here precisely so their number cannot quietly grow.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every file permitted to name `vkGetQueryPoolResults`, repo-relative with `/` separators.
///
/// A file appears here because it either **loads** the entry point, **declares** its signature, or
/// **calls** it — or, for the last three, because it *documents* the blocking contract that is the
/// whole reason this gate exists. Adding a row is a deliberate act: it means somebody decided a
/// new site may read query results, and the review of that decision is the point.
const PINNED: &[&str] = &[
    // The `PFN_` signature.
    "crates/boyko_rhi_vulkan/src/ffi.rs",
    // The loader — one `load_device_command` row.
    "crates/boyko_rhi_vulkan/src/device.rs",
    // The only file that CALLS it: the three FROZEN blocking readers plus rung 4's
    // non-blocking one, all four sharing this file with `GPU_ZONE_QUERY_FLAGS`'s const-assert.
    "crates/boyko_rhi_vulkan/src/rhi_impl/device.rs",
    // Documentation-only mentions of the blocking contract. They call nothing.
    "crates/boyko_rhi/src/device.rs",
    "crates/boyko_app/src/gpu_scene/mod.rs",
    "crates/boyko_app/tests/vb_bench_totality_gate.rs",
    // A module doc naming the FFI call its harness reaches THROUGH `read_query_pool_ns` — a
    // FROZEN blocking reader used correctly (it brackets exactly one pass and reads exactly that
    // pair, after the fence). Found by this gate on its first run, which is the behaviour the
    // enumeration is for: the row exists because somebody looked, not because a grep was tuned
    // until it went quiet.
    "crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs",
    // The replacement's own module doc, explaining what the BLOCK cost — the three collectors it
    // replaces are separate *because* `VK_QUERY_RESULT_WAIT_BIT` makes this call wait forever on a
    // query nobody wrote. It calls nothing; it reads through `read_query_pool_pairs_available`,
    // rung 4's non-blocking verb, whose flag word is const-asserted free of `WAIT_BIT`.
    //
    // ⚠️ THE ROW IS LATE, AND THAT IS THE FINDING. The mention landed with the module at rung 5a
    // (`ee9196b6`) and this gate has been RED from that commit through `7ae9162a` — five commits,
    // three of which certified "workspace green" without ever running it. The gate was right the
    // whole time; nobody asked it. An enumeration that is not executed is a list, not a gate.
    "crates/boyko_rhi_vulkan/src/present/gpu_zone.rs",
    // This gate itself, which names the symbol in order to look for it.
    "tests/gpu_blocking_reader_census.rs",
];

/// The FFI entry point whose every mention is enumerated.
const SYMBOL: &str = "vkGetQueryPoolResults";

/// Directories the walk skips outright.
const SKIP_DIRS: &[&str] = &["target", ".git", "graphify-out", "book", "assets"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, repo-relative, `/`-separated.
fn collect_rs(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_rs(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

fn rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn the_set_of_files_naming_the_blocking_query_reader_is_pinned() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_rs(&root, &root, &mut files);

    // A walk that found nothing would report a triumphant green over an empty set — the vacuous
    // shape this corpus refuses. The floor is well below the tree's real count and exists only to
    // catch a walker that stopped walking.
    assert!(
        files.len() > 200,
        "the source walk found only {} .rs files — the walker is broken, not the tree",
        files.len()
    );

    let mut found: BTreeSet<String> = BTreeSet::new();
    for path in &files {
        let full = root.join(path);
        let Ok(text) = std::fs::read_to_string(&full) else { continue };
        if text.contains(SYMBOL) {
            found.insert(rel(path));
        }
    }

    let pinned: BTreeSet<String> = PINNED.iter().map(|s| (*s).to_string()).collect();

    let new: Vec<&String> = found.difference(&pinned).collect();
    let gone: Vec<&String> = pinned.difference(&found).collect();

    assert!(
        new.is_empty(),
        "G2a: a NEW file names `{SYMBOL}`. A blocking GPU query reader is the one defect this \
         seam exists to make unrepresentable, and its symptom is a HANG, not a failure — so it \
         is caught by enumeration instead. If the new site is genuinely non-blocking, add it to \
         `PINNED` in the same commit that adds the site, and say in the row why. New files: {new:?}"
    );
    assert!(
        gone.is_empty(),
        "G2a: a PINNED file no longer names `{SYMBOL}`. That is good news, but the pin must \
         shrink deliberately — a stale row makes the list read as coverage it no longer has. \
         Drop these rows: {gone:?}"
    );
}

/// The census's own instrument, proved live: the symbol IS present in the tree and the walk DOES
/// find it.
///
/// Without this, a walk that read no file, a `SKIP_DIRS` typo that skipped `crates`, or a symbol
/// name that drifted would all leave `found` empty — and an empty `found` against a non-empty
/// `PINNED` fails on the `gone` clause, which reads as "a file stopped calling it" rather than
/// "the instrument is dead". This says which.
#[test]
fn the_census_instrument_actually_finds_the_symbol() {
    let root = repo_root();
    let caller = root.join("crates/boyko_rhi_vulkan/src/rhi_impl/device.rs");
    let text = std::fs::read_to_string(&caller)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", caller.display()));
    assert!(
        text.contains(SYMBOL),
        "the one file that certainly calls `{SYMBOL}` does not name it — the census has no subject"
    );
    assert!(
        text.contains("GPU_ZONE_QUERY_FLAGS"),
        "G2a's const-asserted flag word is not in the file that calls the reader, so the two \
         halves of the gate are no longer looking at the same code"
    );
}
