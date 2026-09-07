//! KE16 feature-scheme census — the rule that "a feature builds if and only if
//! its arm is in the tree" holds for EVERY switch, not only the ones somebody
//! remembered.
//!
//! ## The failure this exists to catch
//!
//! `KE16-DESIGN.md` §4 makes the witness (`KE16_A/B/W/C`, `ke16_variant()`,
//! `KE16_EXPECT`) the guarantee that "the `--features` line did not take"
//! cannot produce a number. The witness cannot deliver that guarantee alone,
//! because it reports which FEATURE is enabled, not whether that feature's ARM
//! exists: a feature declared in `Cargo.toml` whose behavioural code has not
//! landed produces a binary identical to the default build while
//! `ke16_variant()` prints the candidate's token, so `KE16_EXPECT` certifies
//! the run and a default-build number is filed against the candidate. The
//! number is plausible, so nothing downstream catches it.
//!
//! The tree closes that with one `#[cfg(feature = "…")] compile_error!` per
//! unlanded feature. That closure is six hand-written lines which each axis is
//! expected to delete when it lands its own arm — exactly the shape that rots
//! silently. This census makes the rule mechanical instead:
//!
//! 1. the declared `ke16-*` set is EXACTLY the design's eleven switches, with
//!    `default = []` and the two implied edges;
//! 2. every declared switch either has a behavioural arm (a `feature = "…"`
//!    `cfg` outside `lib.rs`, where the witness consts live) or a SOLO refusal
//!    (`#[cfg(feature = "X")]` directly above a `compile_error!`);
//! 3. the manifest row and the refusal agree: a switch with a solo refusal is
//!    marked `NOT IN THIS TREE` in `Cargo.toml`, and a row so marked has a
//!    refusal. Two carriers of the same claim cannot drift apart.
//! 4. every illegal pair `KE16-DESIGN.md` §4 names has its own
//!    `compile_error!`, so an axis deleting its refusal cannot take an
//!    exclusion with it.
//!
//! ## What this census CANNOT see, stated so nobody reads more into a green
//!
//! Rule 2 asks whether a feature has ANY behavioural arm, not whether the arm
//! is COMPLETE. `ke16-w-gate` was the live example while only half of W-b was
//! in the tree — the `<= 1` push gate in `worker::wake_after_push` without the
//! self-excluding thief-residue cascade — and it satisfied rule 2 through the
//! half it had; only its solo refusal kept it unbuildable, and only rules 3 and
//! 4 kept that refusal honest. The W axis has since landed the cascade
//! (`worker::wake_after_residue`) and deleted the refusal, so no switch is in
//! that state today. Nothing here would notice if one were again: "the arm is
//! complete" is a judgement about the design, not a property of the source
//! text, and no grep decides it.
//!
//! Reading the SOURCE rather than `cfg!()` is deliberate: a `cfg!`-based check
//! can only speak about the configuration it was compiled in, and the property
//! here is about configurations that were NOT built.

use std::fs;
use std::path::{Path, PathBuf};

/// The eleven switches `KE16-DESIGN.md` §4 declares, in the design's order.
const DESIGN_SWITCHES: [&str; 11] = [
    "ke16-a1",
    "ke16-a1-fifo",
    "ke16-a2",
    "ke16-a3",
    "ke16-a5",
    "ke16-b1",
    "ke16-b3",
    "ke16-w-gate",
    "ke16-w-count",
    "ke16-c-batch",
    "ke16-w-fanout",
];

/// The implied-feature edges of §4: `ke16-a5` needs A2's scan set and
/// `ke16-w-fanout` rides on the batch spawn. Everything else is a leaf.
const DESIGN_IMPLIED_EDGES: [(&str, &str); 2] =
    [("ke16-a5", "ke16-a2"), ("ke16-w-fanout", "ke16-c-batch")];

/// The illegal pairs of §4 that are expressible as "both features on":
/// every pair among the four A placements, and `ke16-a5` with each placement
/// it is not the implication of, and the two B joiners against each other.
/// (`ke16-b1`/`ke16-b3` without an A1 arm is a differently shaped rule and is
/// checked separately, because its `cfg` is a `not(any(…))`.)
const DESIGN_ILLEGAL_PAIRS: [(&str, &str); 10] = [
    ("ke16-a1", "ke16-a1-fifo"),
    ("ke16-a1", "ke16-a2"),
    ("ke16-a1", "ke16-a3"),
    ("ke16-a1-fifo", "ke16-a2"),
    ("ke16-a1-fifo", "ke16-a3"),
    ("ke16-a2", "ke16-a3"),
    ("ke16-a5", "ke16-a1"),
    ("ke16-a5", "ke16-a1-fifo"),
    ("ke16-a5", "ke16-a3"),
    ("ke16-b1", "ke16-b3"),
];

/// The marker a `Cargo.toml` row carries while its arm is absent.
///
/// Matched only at the START of a comment line, never anywhere in the block.
/// The `[features]` header explains the convention in prose ("A row marked NOT
/// IN THIS TREE names a candidate whose ARM has not landed yet…") and that
/// header is contiguous with the first switch row, so a substring search over
/// the block would report `ke16-a1` as marked — a gate that fires on the
/// documentation of the rule rather than on the rule.
const ABSENT_MARKER: &str = "NOT IN THIS TREE";

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// One row of the `[features]` table: its dependency list and the comment block
/// directly above it (the rows are documented by the comments that precede
/// them, so a row's claim about itself lives there).
struct FeatureRow {
    name: String,
    deps: Vec<String>,
    preceding_comment: String,
}

/// Parse the `[features]` table of `Cargo.toml`.
///
/// Hand-rolled rather than pulled through a TOML crate: the census must see the
/// COMMENTS above each row (rule 3), which a parsed document discards, and the
/// crate has no TOML dev-dependency to borrow.
fn parse_feature_rows(manifest: &str) -> Vec<FeatureRow> {
    let mut rows = Vec::new();
    let mut in_features = false;
    let mut comment = String::new();

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            comment.clear();
            continue;
        }
        if !in_features {
            continue;
        }
        if trimmed.is_empty() {
            comment.clear();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            comment.push_str(rest.trim());
            comment.push('\n');
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            comment.clear();
            continue;
        };
        // A trailing `# comment` on the row itself is not part of the value.
        let value = value.split('#').next().unwrap_or("").trim();
        let deps = value
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|d| d.trim().trim_matches('"').to_string())
            .filter(|d| !d.is_empty())
            .collect();
        rows.push(FeatureRow {
            name: name.trim().to_string(),
            deps,
            preceding_comment: std::mem::take(&mut comment),
        });
    }
    rows
}

/// Does any source file OTHER than `lib.rs` gate on `feature`?
///
/// `lib.rs` is excluded because it holds the witness consts and the refusals,
/// which mention every feature by construction; a gate anywhere else is a
/// behavioural arm.
fn has_behavioural_arm(feature: &str) -> bool {
    let src = crate_root().join("src");
    let needle = format!("feature = \"{feature}\"");
    fs::read_dir(&src)
        .expect("the crate has a src/ directory")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .filter(|e| e.path().file_name().is_some_and(|n| n != "lib.rs"))
        .any(|e| read(&e.path()).contains(&needle))
}

/// Does `lib.rs` refuse `feature` on its own — `#[cfg(feature = "X")]` directly
/// above a `compile_error!`?
///
/// The "directly above" and "on its own" are both load-bearing. An exclusion
/// (`#[cfg(all(feature = "X", feature = "Y"))]`) also names `X` in its message,
/// so a substring search for the name would report every paired feature as
/// refused; only the solo `cfg` means "this feature alone does not build".
fn has_solo_refusal(lib_rs: &str, feature: &str) -> bool {
    let attr = format!("#[cfg(feature = \"{feature}\")]");
    lib_rs
        .split(&attr)
        .skip(1)
        .any(|after| after.trim_start().starts_with("compile_error!"))
}

#[test]
fn the_declared_switch_set_is_exactly_the_designs_eleven() {
    let manifest = read(&crate_root().join("Cargo.toml"));
    let mut declared: Vec<String> = parse_feature_rows(&manifest)
        .into_iter()
        .map(|r| r.name)
        .filter(|n| n.starts_with("ke16-"))
        .collect();
    declared.sort();
    let mut expected: Vec<String> = DESIGN_SWITCHES.iter().map(|s| s.to_string()).collect();
    expected.sort();

    assert_eq!(
        declared, expected,
        "the `ke16-*` switches declared in Cargo.toml are not the eleven of \
         KE16-DESIGN.md §4; a switch the design does not name has no witness token, so a run \
         built with it reports a variant string that is a lie by omission"
    );
}

#[test]
fn the_default_feature_set_is_empty() {
    let manifest = read(&crate_root().join("Cargo.toml"));
    let rows = parse_feature_rows(&manifest);
    let default = rows
        .iter()
        .find(|r| r.name == "default")
        .expect("Cargo.toml declares a `default` feature row");

    assert!(
        default.deps.is_empty(),
        "`default` enables {:?}; §4 requires `default = []` so the untouched tree measures \
         today's behaviour and every candidate is opt-in",
        default.deps
    );
}

#[test]
fn the_implied_feature_edges_are_the_designs_two() {
    let manifest = read(&crate_root().join("Cargo.toml"));
    let rows = parse_feature_rows(&manifest);

    for row in rows.iter().filter(|r| r.name.starts_with("ke16-")) {
        let expected: Vec<&str> = DESIGN_IMPLIED_EDGES
            .iter()
            .filter(|(from, _)| *from == row.name)
            .map(|(_, to)| *to)
            .collect();
        let actual: Vec<&str> = row.deps.iter().map(String::as_str).collect();
        assert_eq!(
            actual, expected,
            "`{}` implies {actual:?} but §4 gives {expected:?}; an implication the witness arms \
             do not exclude for produces two definitions of the same const, and one the arms DO \
             exclude for silently retires a candidate",
            row.name
        );
    }
}

#[test]
fn every_declared_switch_has_either_an_arm_or_a_refusal() {
    let lib_rs = read(&crate_root().join("src").join("lib.rs"));

    for feature in DESIGN_SWITCHES {
        let arm = has_behavioural_arm(feature);
        let refused = has_solo_refusal(&lib_rs, feature);
        assert!(
            arm || refused,
            "`{feature}` is declared in Cargo.toml, gates no code outside lib.rs, and lib.rs does \
             not refuse it: `--features {feature}` therefore BUILDS a binary identical to the \
             default while `ke16_variant()` reports its token, so `KE16_EXPECT` would certify a \
             default-build number as that candidate's. Land the arm, or add \
             `#[cfg(feature = \"{feature}\")] compile_error!(…)`"
        );
    }
}

#[test]
fn a_refused_switch_and_its_manifest_row_say_the_same_thing() {
    let root = crate_root();
    let lib_rs = read(&root.join("src").join("lib.rs"));
    let manifest = read(&root.join("Cargo.toml"));
    let rows = parse_feature_rows(&manifest);

    for feature in DESIGN_SWITCHES {
        let row = rows
            .iter()
            .find(|r| r.name == feature)
            .unwrap_or_else(|| panic!("`{feature}` has no row in Cargo.toml"));
        let refused = has_solo_refusal(&lib_rs, feature);
        let marked = row
            .preceding_comment
            .lines()
            .any(|l| l.starts_with(ABSENT_MARKER));

        assert_eq!(
            refused,
            marked,
            "`{feature}`: lib.rs {} it, Cargo.toml's row {} it `{ABSENT_MARKER}`. The manifest row \
             and the refusal are two carriers of one claim — whether this candidate can be \
             measured — and a reader who consults only the row would be told the wrong thing",
            if refused {
                "refuses"
            } else {
                "does not refuse"
            },
            if marked { "marks" } else { "does not mark" },
        );
    }
}

#[test]
fn every_illegal_pair_of_the_design_has_its_own_compile_error() {
    let lib_rs = read(&crate_root().join("src").join("lib.rs"));

    for (left, right) in DESIGN_ILLEGAL_PAIRS {
        // Either order is accepted: the pair is the claim, not the spelling.
        let forward = format!("#[cfg(all(feature = \"{left}\", feature = \"{right}\"))]");
        let backward = format!("#[cfg(all(feature = \"{right}\", feature = \"{left}\"))]");
        let present = [forward, backward].iter().any(|attr| {
            lib_rs
                .split(attr.as_str())
                .skip(1)
                .any(|after| after.trim_start().starts_with("compile_error!"))
        });
        assert!(
            present,
            "no `compile_error!` guards the illegal pair (`{left}`, `{right}`): \
             `--features {left},{right}` would build ONE binary that the witness names with both \
             candidates' tokens, and the tournament would compare it against itself"
        );
    }
}

// =========================================================================
// Calibration — the census's two text predicates, driven over synthetic input
// in BOTH polarities.
//
// Every assertion above is of the form "the tree satisfies P". A green there
// means nothing unless P can also be false: a predicate that answers `true`
// for any input, or a parser that silently yields an empty row set, would make
// the whole file a gate that cannot fail. These two tests are the calibration
// copies, and they are the reason the census is allowed to be believed.
// =========================================================================

#[test]
fn the_solo_refusal_predicate_distinguishes_a_refusal_from_an_exclusion() {
    // An exclusion names the feature in its message and gates on the PAIR.
    // Treating that as a refusal is the mistake that would report every paired
    // feature as unbuildable and pass the census over a tree with no refusals
    // at all.
    let exclusion_only = r#"
#[cfg(all(feature = "ke16-x", feature = "ke16-y"))]
compile_error!("axis X: `ke16-x` and `ke16-y` are mutually exclusive");
"#;
    assert!(
        !has_solo_refusal(exclusion_only, "ke16-x"),
        "an exclusion on the pair was read as a refusal of `ke16-x` alone"
    );

    // A solo `cfg` directly above the macro IS the refusal.
    let solo = r#"
#[cfg(feature = "ke16-x")]
compile_error!("axis X: `ke16-x` is declared but NOT IMPLEMENTED in this tree");
"#;
    assert!(
        has_solo_refusal(solo, "ke16-x"),
        "the solo refusal of `ke16-x` was not recognised"
    );

    // A solo `cfg` on something that is not a `compile_error!` is an ARM.
    let arm = r#"
#[cfg(feature = "ke16-x")]
pub const KE16_X: &str = "x";
"#;
    assert!(
        !has_solo_refusal(arm, "ke16-x"),
        "a witness const under a solo cfg was read as a refusal"
    );
}

#[test]
fn the_manifest_parser_attributes_a_comment_block_to_the_row_below_it() {
    let manifest = r#"
[package]
name = "irrelevant"

[features]
default = []
# NOT IN THIS TREE (axis Q): the arm is missing.
ke16-q = []

ke16-r = ["ke16-q"]
[lints]
"#;
    let rows = parse_feature_rows(manifest);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["default", "ke16-q", "ke16-r"],
        "the parser did not read exactly the rows of the `[features]` table"
    );

    let q = &rows[1];
    assert!(
        q.preceding_comment
            .lines()
            .any(|l| l.starts_with(ABSENT_MARKER)),
        "the comment directly above `ke16-q` was not attributed to it"
    );

    let r = &rows[2];
    assert_eq!(
        r.deps,
        vec!["ke16-q".to_string()],
        "the implied-feature list of `ke16-r` was not parsed"
    );
    assert!(
        r.preceding_comment.is_empty(),
        "a blank line must detach a comment block: `ke16-r` inherited `ke16-q`'s marker, which \
         would report an implemented feature as absent"
    );
}

#[test]
fn the_b_joiners_are_refused_without_an_a1_placement_arm() {
    let lib_rs = read(&crate_root().join("src").join("lib.rs"));

    // The rule is shaped `any(b1, b3) AND NOT any(a1, a1-fifo)`, so it is
    // matched by its distinguishing clause rather than by a pair.
    let guard = "not(any(feature = \"ke16-a1\", feature = \"ke16-a1-fifo\"))";
    assert!(
        lib_rs.contains(guard),
        "lib.rs carries no `{guard}` clause: `ke16-b1` / `ke16-b3` would build over A2/A3/A5, \
         where the worker joiner has no REGISTERED destination deque and silently stays at B0 \
         while the witness reports b1/b3"
    );
    assert!(
        lib_rs.contains("`ke16-b1` / `ke16-b3` require"),
        "the B-without-A1 refusal exists but its message does not name the requirement; \
         §4 asks each `compile_error!` to name the features and the axis"
    );
}
