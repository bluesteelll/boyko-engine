//! KE16 App-1 — the three physics dispatch sites take their lane count from `num_threads()`, and
//! no site reintroduces the `+ 1`.
//!
//! ## Why a census and not a behavioural test
//!
//! App-1 (`docs/threadpool/KE16-DESIGN-APP.md` §1) changed three sites from
//! `pool.num_threads() + 1` to `pool.num_threads()`: `solver/colored.rs`, `soft/colored.rs` and
//! `resources.rs`. The `+ 1` was the bench route's external joiner counted as a production lane;
//! on the shipping route the joiner is one OF the W workers, so the extra lane never existed.
//!
//! The change is deliberately INVISIBLE to every behavioural gate in the corpus, and that is the
//! problem this file exists for:
//!
//! * The `{1, N}` bit-identity oracles cannot see it — the design's own argument is that the
//!   candidate set is chunk-count- and chunk-shape-independent, which is exactly why those oracles
//!   stayed green across the edit (`tests/ke16_app1_solve_in_system_bit_identity.rs`,
//!   `tests/broadphase_grid.rs::parallel_dispatched_bit_identical_at_2_3_4_workers`).
//! * The allocation bound in `tests/colored_parallel_alloc_o6.rs` is generous by construction
//!   (`16 + workers × CHUNKS_PER_WORKER × 5` per scope, and its load-bearing claim is
//!   n_colors-INDEPENDENCE, not the absolute number), so at W=4 it admits 24 and 30 chunk closures
//!   alike. Re-adding the `+ 1` does not red it.
//! * Wall-clock cannot see one chunk in 96.
//!
//! So a reintroduced `+ 1` — the single most likely regression of this item, because the old
//! rationale reads plausibly and three sites must agree — would ship green. A source census is
//! what the repo already uses for exactly this class (`tests/isa_baseline_census.rs`,
//! `crates/boyko_ecs/tests/ke16_witness_census.rs`, `solver/simd.rs`'s no-FMA census): the property
//! is a property OF THE SOURCE, so the gate reads the source.
//!
//! ## What the scan is, precisely
//!
//! Comments are stripped before matching (each line is cut at its first `//`), because the three
//! sites are surrounded by prose that quotes the retired spelling — "the lane pool is
//! `num_threads()`, never `+ 1`" — and a scanner that counted those would fail on correct code and
//! be silenced, which is worse than not existing. Whitespace is then removed, so `+ 1`, `+1` and a
//! line-wrapped `+\n1` all match one pattern.
//!
//! [`the_census_predicate_reports_an_injected_plus_one_and_ignores_prose`] runs both directions of
//! the predicate over synthetic input, so a scan that silently stopped matching anything is not
//! reported as a clean tree.

use std::fs;
use std::path::PathBuf;

/// The three sites App-1 names, relative to this crate's manifest directory.
const APP1_SITES: [&str; 3] = [
    "src/solver/colored.rs",
    "src/soft/colored.rs",
    "src/resources.rs",
];

/// The lane-count binding every App-1 site must spell, whitespace-removed.
const LANE_BINDING: &str = "letlanes=pool.num_threads();";

/// The retired spelling, whitespace-removed.
const RETIRED_PLUS_ONE: &str = "num_threads()+1";

/// Everything in `src`, as (path, code-only text) pairs.
///
/// Code-only: each line is cut at its first `//`. That is a lexer's approximation — it would also
/// cut a `//` inside a string literal — and it is the safe direction here: cutting too much can
/// only HIDE a match, and the calibration test below pins that a real `+ 1` in code is still seen.
fn physics_sources() -> Vec<(PathBuf, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("invariant: {} must be readable ({e})", dir.display()));
        for entry in entries {
            let entry = entry.expect("invariant: a readable directory yields readable entries");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("invariant: {} must be readable ({e})", path.display()));
                out.push((path, strip_comments(&text)));
            }
        }
    }
    out
}

/// Drops every `//`-comment tail and ALL whitespace — the line breaks included — leaving a string
/// in which `+ 1`, `+1` and a rustfmt-wrapped `num_threads()` / `+ 1` pair are one text.
///
/// Joining the lines is what makes the wrapped form visible, and it cannot manufacture a match
/// across two statements: for `num_threads()+1` to appear spuriously a line would have to END at
/// exactly `num_threads()` and the next one BEGIN with `+ 1`, which is that wrapped expression
/// itself. The calibration test pins both directions.
fn strip_comments(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    for line in text.lines() {
        let head = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        code.extend(head.chars().filter(|c| !c.is_whitespace()));
    }
    code
}

/// Each App-1 site binds its lane count from `num_threads()`.
///
/// What it loses to: a site that goes back to `num_threads() + 1`, or one that stops reading the
/// pool at all (a hard-coded lane count would pass the negative test below and fail this one).
#[test]
fn every_app1_site_binds_the_lane_count_from_num_threads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for site in APP1_SITES {
        let path = root.join(site);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("invariant: the App-1 site {site} must exist ({e})"));
        let code = strip_comments(&text);
        assert!(
            code.contains(LANE_BINDING),
            "{site} no longer contains `let lanes = pool.num_threads();`. KE16 App-1 fixes the \
             lane count of all THREE physics dispatch sites at `num_threads()`; a site that binds \
             it differently is either the retired `+ 1` returning or a site that stopped asking \
             the pool, and no bit-identity oracle or allocation bound in this crate can see either \
             (see this file's header)"
        );
    }
}

/// No file in `boyko_physics`'s source computes `num_threads() + 1` in CODE.
///
/// Wider than the three sites on purpose: the defect class is "a lane count that counts the
/// external joiner as a production lane", and a fourth dispatch site added later would carry it in
/// just as quietly. The prose that quotes the retired spelling is not matched (comments are cut).
#[test]
fn no_physics_source_reintroduces_the_plus_one_lane() {
    let offenders: Vec<String> = physics_sources()
        .into_iter()
        .filter(|(_, code)| code.contains(RETIRED_PLUS_ONE))
        .map(|(path, _)| path.display().to_string())
        .collect();

    assert!(
        offenders.is_empty(),
        "these files compute `num_threads() + 1` in code: {offenders:?}. On the production route \
         the thread that calls `pool.scope` is one OF the W workers (KE16-DESIGN-APP.md §1), so \
         the extra lane does not exist and the site over-chunks every wave by CHUNKS_PER_WORKER \
         chunks. On the bench route the external joiner IS an extra lane, but its share is \
         measured, not counted"
    );
}

/// The predicate can report, and can stay silent — checked over synthetic input, not over the tree.
///
/// Without this, a `strip_comments` that returned the empty string (or a pattern that stopped
/// matching after a formatting change) would make both tests above pass over any tree at all.
#[test]
fn the_census_predicate_reports_an_injected_plus_one_and_ignores_prose() {
    let injected = "        let lanes = pool.num_threads() + 1;\n";
    assert!(
        strip_comments(injected).contains(RETIRED_PLUS_ONE),
        "the scan cannot see a reintroduced `+ 1` in code — the two tests above are vacuous"
    );

    let wrapped = "        let lanes = pool.num_threads()\n            + 1;\n";
    assert!(
        strip_comments(wrapped).contains(RETIRED_PLUS_ONE),
        "the scan cannot see a line-wrapped `+ 1` — rustfmt would hide the regression"
    );

    let prose = "        // KE16 App-1: the lane pool is `num_threads() + 1` no longer.\n";
    assert!(
        !strip_comments(prose).contains(RETIRED_PLUS_ONE),
        "the scan counts a COMMENT that quotes the retired spelling; it would be red on correct \
         code, and a census that is red on correct code gets silenced"
    );

    let binding = "        let lanes = pool.num_threads();\n";
    assert!(
        strip_comments(binding).contains(LANE_BINDING),
        "the scan cannot see the lane binding it requires — the site test above is vacuous"
    );
}
