//! KE16 M2w — the MEMBERSHIP census over the negative control's eight pieces.
//!
//! # What this file claims, and the much larger thing it does not
//!
//! It claims that the pieces of the M2w negative arm are IN THE TREE. It claims
//! **nothing whatever about the property** — not that the arm is UB, not that
//! Tree Borrows catches it, not that the shipped `run_scoped` is sound. Those are
//! decided by execution, by `scripts/tb_neg_gate.ps1` and by the positive gate
//! `tests/miri_scope_completion_protector.rs`, and no amount of source text can
//! stand in for either.
//!
//! What it exists for is one sentence: **landing the block without the arm must
//! be red at census time, not at review time.** Stage 3b deleted the scoped
//! path's strongest soundness clause (the cell was freed before the body ran, so
//! at the release RMW the payload allocation did not exist at all) and replaced
//! it with a reduction to one function plus an execution gate over that
//! function. A tree that carries the deletion and not the gate has shipped a
//! soundness downgrade with a positive-only column, and nothing in `cargo test`
//! would have said so. This census says so.
//!
//! # Why it reads SOURCE TEXT rather than `cfg!()`
//!
//! Because the property it guards is about configurations that were NOT built. A
//! `cfg!`-based check can only speak about the configuration it was compiled in,
//! and every piece here belongs to a build (`--features tb-neg-m2w`, under Miri)
//! that a plain `cargo test` never produces — indeed one that `src/lib.rs`
//! REFUSES to produce natively, on purpose. Reading the text is a deliberate
//! choice, stated here so nobody mistakes it for laziness.
//!
//! `cargo check --features tb-neg-m2w` "must fail" cannot be the only guard the
//! row has — an exit code cannot tell a REFUSAL from an ABSENCE, and before the
//! feature was declared that condition was green because cargo did not know the
//! feature at all.
//!
//! # Membership only, and no counts
//!
//! Deliberately no "the arm is N lines", no "the driver runs exactly four
//! processes", no token census over spellings that mint a reference. The last of
//! those was designed, reviewed and DELETED: a rule claiming to enumerate the
//! ways Rust mints a reference cannot be made exhaustive (binding patterns,
//! whitespace, macro autoref, `assert_eq!`'s `match (&a, &b)`, operator autoref,
//! RFC 2229 capture, `#[derive]`), and it was banning the wrong thing anyway —
//! under Tree Borrows an unprotected reference may cover freed memory, and only
//! a reference that is a live call's ARGUMENT is forbidden to. A census that
//! overclaims is worse than no census, so this one is scoped to the question
//! source text can actually answer: is each piece present?
//!
//! # Everything here is gated on `src/block.rs` existing
//!
//! The arm exists to control for the block. On a tree without `src/block.rs`
//! there is no `free_all`, no chunk class, and nothing for a protector to span —
//! so the census is inert rather than red. That is the ONE conditional in the
//! file, and it is a scope statement, not an escape hatch: `src/block.rs` is in
//! the tree today.
//!
//! # The receipts are RED UNTIL THE MIRI LEG IS RUN, and that is correct
//!
//! `receipts_for_every_seed_exist_and_name_the_declared_diagnostic` fails on a
//! tree where `scripts/tb_neg_gate.ps1` has never been executed. That is not a
//! broken gate; it is the gate reporting that KE16 exit condition 6 has not been
//! discharged. Its message says so in those words, because "an un-run gate" and
//! "a broken gate" look identical from a red line and this repository has been
//! bitten by the confusion before.

use std::fs;
use std::path::{Path, PathBuf};

/// The feature that swaps `run_scoped_neg::<F>` into `Task::execute`.
const FEATURE: &str = "tb-neg-m2w";

/// The exact attribute the arm's items must carry, spelled as rustfmt writes it.
const CFG_ATTR: &str = "#[cfg(feature = \"tb-neg-m2w\")]";

/// The four seeds of the measured receiver × seed table in
/// `tests/miri_scope_completion_protector.rs`'s module header. The driver runs
/// one process per seed; `-Zmiri-many-seeds` demonstrates the seed-independence
/// of a GREEN and is meaningless for a run whose success is an abort.
const SEEDS: [u32; 4] = [0, 1, 7, 15];

/// The driver test binary, relative to the crate root.
const DRIVER_TEST: &str = "tests/tb_neg_m2w_block_reference.rs";

/// The two carriers of the driver recipe, relative to the repository root. Both
/// are censused: a change to one that skips the other is exactly the drift a
/// single-file check would miss.
const DRIVER_SCRIPTS: [&str; 2] = ["scripts/tb_neg_gate.ps1", "scripts/tb_neg_gate.sh"];

/// Where `scripts/tb_neg_gate.*` writes one receipt per seed, relative to the
/// repository root.
const RECEIPT_DIR: &str = "docs/threadpool/receipts";

/// The arm's protector site, spelled as source text rather than as a line.
///
/// Tree Borrows installs the protector at the fn-entry retag of this reference
/// parameter, so this exact substring is what the receipt's `the protected tag …
/// was created here` location points at. Both driver scripts derive the same
/// line from the same needle, and `both_driver_scripts_name_the_arm_and_the_four_seeds`
/// requires them to carry it, so the three readers cannot drift apart.
const ARM_SIGNATURE: &str = "_cell: &ScopedCell<";

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The repository root, derived and then CHECKED.
///
/// `CARGO_MANIFEST_DIR` is `<root>/crates/boyko_threadpool`, so the root is two
/// parents up — and a derivation that is merely plausible is how a census ends up
/// asserting over a directory that does not exist and calling the resulting
/// "file not found" a finding. The self-consistency check below is what makes a
/// wrong root a loud panic instead of a misattributed red.
fn repo_root() -> PathBuf {
    let root = crate_root()
        .parent()
        .and_then(Path::parent)
        .expect("invariant: CARGO_MANIFEST_DIR is <repo>/crates/boyko_threadpool")
        .to_path_buf();

    assert!(
        root.join("crates")
            .join("boyko_threadpool")
            .join("Cargo.toml")
            .is_file(),
        "the repository root derived from CARGO_MANIFEST_DIR is {}, which does not contain \
         crates/boyko_threadpool/Cargo.toml. Every assertion about scripts/ and docs/ below \
         would be reporting on the wrong tree",
        root.display()
    );

    root
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The attribute-and-doc block directly above the first line containing
/// `item_needle`, or `None` if the item is absent.
///
/// The backward walk stops at the first line that is neither blank nor a comment
/// nor an attribute, so an attribute belonging to some PREVIOUS item is never
/// collected — "under the cfg" means the attribute block of THIS item and
/// nothing else. A plain substring search for the feature name is satisfied by
/// any mention anywhere in the file, including the comment that explains the
/// rule, which is what makes "directly above" load-bearing rather than tidy.
fn attribute_block_of<'a>(text: &'a str, item_needle: &str) -> Option<Vec<&'a str>> {
    let lines: Vec<&str> = text.lines().collect();
    let idx = lines.iter().position(|l| l.contains(item_needle))?;

    let mut block = Vec::new();
    for line in lines[..idx].iter().rev() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("#[") || t.starts_with("//") {
            block.push(t);
            continue;
        }
        break;
    }
    Some(block)
}

/// Is `item_needle` carried by an item whose attribute block contains `CFG_ATTR`?
fn item_is_gated_by_the_feature(text: &str, item_needle: &str) -> bool {
    attribute_block_of(text, item_needle).is_some_and(|b| b.contains(&CFG_ATTR))
}

/// Does `[features]` declare a row named `name`?
///
/// Hand-rolled rather than pulled through a TOML crate: the crate has no TOML
/// dev-dependency, and one row's presence does not justify adding one.
fn manifest_declares_feature(manifest: &str, name: &str) -> bool {
    let mut in_features = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_features = t == "[features]";
            continue;
        }
        if !in_features || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = t.split_once('=')
            && key.trim() == name
        {
            return true;
        }
    }
    false
}

/// Whitespace-stripped haystack, so a check for "this attribute, directly above
/// this macro" survives rustfmt splitting the attribute across lines.
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Lines whose trimmed form STARTS WITH `prefix`.
///
/// The line-start anchor is the whole point, and it is why both source-shape
/// checks below use it: a real attribute sits at column 0, while a doc comment
/// that MENTIONS one — as the driver's header does, deliberately, to document
/// the `running 0 tests` trap — is preceded by `//!`. An unanchored substring
/// search cannot tell those apart, and would red the very file that explains why
/// the attribute is absent.
fn count_lines_starting_with(text: &str, prefix: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with(prefix))
        .count()
}

/// Is `src/block.rs` in this tree? Everything below is scoped to that.
fn block_is_in_the_tree() -> bool {
    crate_root().join("src").join("block.rs").is_file()
}

#[test]
fn the_arm_and_its_completer_live_in_task_scoped_under_the_feature() {
    if !block_is_in_the_tree() {
        return;
    }
    let path = crate_root().join("src").join("task").join("scoped.rs");
    let text = read(&path);

    // `run_scoped_neg` is what `Task::new_scoped` puts in `Task::execute`; it is
    // the reduced set's second member and the deliberate counterexample to D5.
    assert!(
        item_is_gated_by_the_feature(&text, "fn run_scoped_neg"),
        "{}: no `fn run_scoped_neg` carrying `{CFG_ATTR}` in its attribute block. The M2w \
         negative arm is what makes the positive gate's green mean anything: without it, \
         `tests/miri_scope_completion_protector.rs` passing is consistent with the deciding \
         interleaving never having happened. Land the arm beside `run_scoped`. (If the arm was \
         instead gated by an enclosing `#[cfg(feature = \"{FEATURE}\")] mod`, WIDEN this census \
         rather than deleting it — the claim it carries is still owed.)",
        path.display()
    );

    // `finish_neg` is the function that actually holds the protector: Tree
    // Borrows installs one on a reference-typed ARGUMENT, so the arm needs a
    // callee to pass the reference to, and it must not be inlined away.
    assert!(
        item_is_gated_by_the_feature(&text, "fn finish_neg"),
        "{}: no `fn finish_neg` carrying `{CFG_ATTR}` in its attribute block. `run_scoped_neg` \
         alone forms a reference into chunk memory but installs NO protector — under Tree Borrows \
         a reference that is created and used and never passed as a call argument does not forbid \
         deallocation. `finish_neg(_cell: &ScopedCell<F>, shared: *const ScopeShared)` is the \
         protector, and `scripts/tb_neg_gate.*` pins the receipt on its name",
        path.display()
    );

    // Scoped to `finish_neg`'s OWN attribute block, not to the file: a
    // whole-file search for `#[inline(never)]` is satisfied by the attribute on
    // ANY item, so it would stop measuring the one frame that has to survive.
    let finish_attrs = attribute_block_of(&text, "fn finish_neg").unwrap_or_default();
    assert!(
        finish_attrs.contains(&"#[inline(never)]"),
        "{}: `fn finish_neg`'s attribute block does not carry `#[inline(never)]`. An inlined \
         `finish_neg` has no frame, so there is no call for a protector to be live across and \
         the negative control goes GREEN — the one failure mode of a negative control that reads \
         as success",
        path.display()
    );
}

#[test]
fn the_manifest_declares_the_feature() {
    if !block_is_in_the_tree() {
        return;
    }
    let path = crate_root().join("Cargo.toml");
    let manifest = read(&path);

    assert!(
        manifest_declares_feature(&manifest, FEATURE),
        "{}: `[features]` does not declare `{FEATURE}`. Until it does, \
         `cargo check -p boyko-threadpool --features {FEATURE}` fails with \"none of the selected \
         packages contains these features\" — non-zero, which satisfies KE16 exit condition 9's \
         literal \"must FAIL\" criterion while the arm it gates does not exist. An exit code \
         cannot tell a refusal from an absence; the declared row is what makes the refusal the \
         reason",
        path.display()
    );
}

#[test]
fn lib_refuses_the_feature_outside_miri() {
    if !block_is_in_the_tree() {
        return;
    }
    let path = crate_root().join("src").join("lib.rs");
    let squeezed = squeeze(&read(&path));

    // Both argument orders, because `all(…)` is a set and rustfmt does not
    // reorder it: pinning one order would make the census a spelling test.
    let refusals = [
        format!("#[cfg(all(feature=\"{FEATURE}\",not(miri)))]compile_error!"),
        format!("#[cfg(all(not(miri),feature=\"{FEATURE}\"))]compile_error!"),
    ];

    assert!(
        refusals.iter().any(|r| squeezed.contains(r.as_str())),
        "{}: no `#[cfg(all(feature = \"{FEATURE}\", not(miri)))] compile_error!(…)`. Without it \
         `--all-features` BUILDS the arm — deliberate Undefined Behavior shipped into the normal \
         test suite, where it would abort or corrupt a run that never asked for it. The refusal \
         is also the only thing that makes KE16 exit condition 9 mean anything now that the \
         feature is declared",
        path.display()
    );
}

#[test]
fn the_driver_binary_has_exactly_one_test_and_no_file_scope_cfg() {
    if !block_is_in_the_tree() {
        return;
    }
    let path = crate_root()
        .join("tests")
        .join("tb_neg_m2w_block_reference.rs");
    assert!(
        path.is_file(),
        "{DRIVER_TEST} does not exist. It is the arm's driver: the one place the deciding \
         interleaving is forced with the arm in `Task::execute`"
    );
    let text = read(&path);

    let tests = count_lines_starting_with(&text, "#[test]");
    assert_eq!(
        tests, 1,
        "{DRIVER_TEST} declares {tests} test attributes at line start; it must declare exactly \
         one. The driver's whole contract is that every configuration prints `running 1 test` and \
         then says out loud what it decided — a second test would make the receipt ambiguous \
         about which one aborted the interpreter, and zero would make the binary a vacuous pass"
    );

    let file_scope_cfgs = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("#![") && t.contains("cfg(")
        })
        .count();
    assert_eq!(
        file_scope_cfgs, 0,
        "{DRIVER_TEST} carries {file_scope_cfgs} file-scope `cfg` attribute(s) at line start. \
         That is this repository's catalogued trap: a file-scope `cfg` that is false prints \
         `running 0 tests` and exits 0 — a vacuous pass indistinguishable from a pass. The \
         configuration checks belong INSIDE the test body, where a wrong configuration is a red \
         with a message. (Mentions inside `//!` prose are fine and intended; this count is \
         anchored at line start for exactly that reason)"
    );

    assert!(
        text.contains("cfg!(miri)"),
        "{DRIVER_TEST} does not contain its `cfg!(miri)` guard. Natively there is no Tree-Borrows \
         protector to hold and no forced schedule to hold it across, so a run outside the gate's \
         configuration decides nothing — and must say so rather than read as a pass"
    );

    assert!(
        text.contains("tb-neg-m2w: arm not built"),
        "{DRIVER_TEST} does not carry the `tb-neg-m2w: arm not built` panic. Without it a build \
         WITHOUT the feature runs the driver, finds no arm to catch, and passes — the absence of \
         the arm reading as its presence"
    );

    // The driver is green in NO configuration — its success is an abort — so it
    // must not sit in the default suite as a permanent red. That is this
    // repository's most-measured defect: a known-red target teaches readers to
    // skip a line, and three reds hid behind one for 87 commits. The sanctioned
    // cure is already used twice in this tree for tests that are RED BY DESIGN
    // (`brick_field_is_conservative_lower_bound` and its sibling): an `#[ignore]`
    // carrying its reason, which `tests/ignore_reasons_census.rs` refuses to let
    // be bare or empty.
    //
    // Censused here, and not left to that census, because the two gates decide
    // different things: the ignore census asks whether SOME reason is written,
    // and this asks whether THIS driver is excluded on exactly the configuration
    // it cannot decide in. Both halves matter — a bare `not(miri)` would leave a
    // plain `cargo miri test` running the arm-not-built panic, which is equally
    // undecidable and equally red.
    // Needles are spelled WITHOUT whitespace because `squeeze` strips all of it —
    // that is what makes the check survive rustfmt splitting the attribute across
    // lines, and it is also the way to get a false red here.
    let squeezed = squeeze(&text);
    assert!(
        squeezed.contains("#[cfg_attr(not(all(miri,feature=\"tb-neg-m2w\")),ignore=\""),
        "{DRIVER_TEST} does not gate its `#[ignore]` on `not(all(miri, feature = \"tb-neg-m2w\"))` \
         with a reason attached. The driver decides its property in exactly one configuration — \
         Miri interpreting the arm — and is green in none, so any other configuration must SKIP it \
         with a written reason rather than stand permanently red in `cargo test --workspace`. \
         A bare `ignore`, or one gated on `miri` alone, fails this: the second would leave a plain \
         `cargo miri test` running the arm-not-built panic, which is equally undecidable"
    );
    assert!(
        text.contains("--include-ignored"),
        "{DRIVER_TEST} does not name `--include-ignored`. Its own ignore attribute makes that flag \
         load-bearing for the driver scripts, and the file must say so where a reader editing the \
         attribute will see it"
    );
}

#[test]
fn both_driver_scripts_name_the_arm_and_the_four_seeds() {
    if !block_is_in_the_tree() {
        return;
    }
    let root = repo_root();

    for rel in DRIVER_SCRIPTS {
        let path = root.join(rel);
        assert!(
            path.is_file(),
            "{rel} does not exist. KE16 exit condition 6 is discharged by running it, and a \
             condition with no invocation decays into one that is never run"
        );
        let text = read(&path);

        for needle in [FEATURE, "run_scoped_neg", "tb_neg_m2w_block_reference"] {
            assert!(
                text.contains(needle),
                "{rel} does not name `{needle}`. The driver must name the feature it enables, \
                 the arm whose protector it pins the receipt on, and the binary it runs; a \
                 driver that lost one of the three is running or checking something else"
            );
        }

        // Both flags are non-optional recipe elements, and neither is a
        // preference. Stacked Borrows does not install the protector at all;
        // and at the default preemption rate the equivalent defect is caught on
        // a MINORITY of seeds (1/4 measured, against 4/4 at rate 0).
        for flag in ["-Zmiri-tree-borrows", "-Zmiri-preemption-rate=0"] {
            assert!(
                text.contains(flag),
                "{rel} does not pass `{flag}`. It is part of this gate's recipe rather than a \
                 tuning preference, and a run without it can be green while the arm is UB"
            );
        }

        // `--include-ignored` became load-bearing when the driver took its
        // `#[cfg_attr(…, ignore = …)]`. Its absence does NOT fail quietly — the
        // binary would report `1 ignored`, no seed would be red, and the gate
        // would say `red on 0/4` — but the diagnosis would point at the property
        // rather than at the flag, which is the expensive kind of red.
        assert!(
            text.contains("--include-ignored"),
            "{rel} does not pass `--include-ignored`. The driver skips itself in every \
             configuration except this gate's, so without the flag the run reports `1 ignored`, \
             produces no receipt, and the gate reads `red on 0/4` as if the property had failed"
        );

        // The receipt rule must be DERIVED from the source, never written down.
        // The first four receipts refuted the previous rule, which required the
        // arm's function NAME in the rendered window: Miri prints these `help:`
        // sub-diagnostics as a location line with no snippet, so the name is not
        // in the receipt at all and the predicate could not match on 4/4 seeds
        // that were red for exactly the declared reason.
        assert!(
            text.contains("_cell: &ScopedCell<"),
            "{rel} does not derive the protector site from `_cell: &ScopedCell<`. The receipt \
             names a location and not a function, so the gate identifies the arm by the line its \
             own source puts the signature on — computed on every run, because a line number \
             written down in another file is the citation shape that rots"
        );

        // Spelled, not parsed: this census reads the scripts as text, so the
        // seed list can only be seen in the one spelling each script pins. Both
        // files carry a comment saying so next to the literal.
        let seed_literal = if rel.ends_with(".ps1") {
            "@(0, 1, 7, 15)"
        } else {
            "(0 1 7 15)"
        };
        assert!(
            text.contains(seed_literal),
            "{rel} does not spell its seed list as `{seed_literal}`. The four seeds are the ones \
             the positive gate's measured receiver × seed table uses, so a red here and a green \
             there are comparable; and the spelling is pinned because this census reads the \
             script as text. Change both carriers together"
        );
    }

    // NOT censused, deliberately: that the scripts do not USE
    // `-Zmiri-many-seeds`. Both name it in prose, to record why it is absent —
    // it demonstrates the seed-independence of a GREEN and would end the process
    // on the first abort, turning "4/4 red" into "at least 1/4 red" and calling
    // that a pass. A "must not contain" check would fire on the documentation of
    // the rule rather than on a violation of it.
}

#[test]
fn receipts_for_every_seed_exist_and_name_the_declared_diagnostic() {
    if !block_is_in_the_tree() {
        return;
    }
    let dir = repo_root().join(RECEIPT_DIR);

    assert!(
        dir.is_dir(),
        "{RECEIPT_DIR}/ does not exist, which means `scripts/tb_neg_gate.ps1` HAS NOT BEEN RUN. \
         This is an UN-RUN gate, not a broken one: KE16 exit condition 6 (4/4 seeds red with \
         receipts) is undischarged, and the M2w execution gate that replaces the soundness clause \
         stage 3b deleted has produced no evidence yet. Run the driver and commit the four \
         receipts"
    );

    for seed in SEEDS {
        let path = dir.join(format!("tb-neg-m2w-{seed}.stderr"));
        assert!(
            path.is_file(),
            "{RECEIPT_DIR}/tb-neg-m2w-{seed}.stderr is missing. Every one of the four seeds must \
             be red: a partial result is a FAILURE, not a pass with a note, because \"red on 2 of \
             4\" is the signature of an unarmed recipe rather than of a sound one"
        );

        let text = read(&path);
        assert!(
            !text.trim().is_empty(),
            "{RECEIPT_DIR}/tb-neg-m2w-{seed}.stderr is empty. An empty receipt is the shape a \
             run that never launched leaves behind — a build or link failure exits 1 exactly \
             like a red gate does, and the two are indistinguishable unless the receipt is read. \
             Until 2026-09-10 this box had no MSVC linker at all, so a `cargo` run that reached \
             the msvc nightly died there; msvc links as of that date, but the reason to read \
             the receipt rather than the exit code is unchanged"
        );

        assert!(
            text.contains("error: Undefined Behavior:"),
            "{RECEIPT_DIR}/tb-neg-m2w-{seed}.stderr contains no `error: Undefined Behavior:`. The \
             negative control's success IS the abort; a receipt without one records a run that \
             failed for some other reason, or a negative control that has stopped being negative"
        );

        // The DECLARED kind, so that a red for a different reason — a data race,
        // an alignment fault, a use-after-free elsewhere — cannot discharge this
        // gate. Two substrings rather than a regex: the crate has no regex
        // dev-dependency and the tag between them is a run-specific number.
        assert!(
            text.contains("deallocation through") && text.contains("is forbidden"),
            "{RECEIPT_DIR}/tb-neg-m2w-{seed}.stderr does not carry the declared diagnostic kind, \
             `deallocation through <TAG> … is forbidden`. That message IS the protector rule; any \
             other UB kind means this run reported something else and says nothing about M2w"
        );

        // ⚠ THIS CHECK ASKED FOR A FUNCTION NAME UNTIL 2026-09-08, AND THE FIRST
        // FOUR RECEIPTS REFUTED IT. The reasoning was sound as far as it went —
        // Tree Borrows installs the protector at the fn-entry retag of a
        // reference-typed PARAMETER, so the span printed for "the protected tag …
        // was created here" is the CALLEE's parameter, `finish_neg`'s `_cell` —
        // but it assumed rustc would render that span WITH its source snippet.
        // Miri prints these two `help:` sub-diagnostics as a bare location line:
        // no gutter, no snippet, and therefore no function name anywhere in the
        // receipt (`grep -c finish_neg` = 0 on all four). The predicate could not
        // match on four seeds that were red for exactly the declared reason —
        // this repository's "gate that cannot pass", the mirror of its usual one.
        //
        // The location IS printed, so identity is checked on that instead, with
        // the line DERIVED from the arm's own source on every run. That is not
        // the citation shape the old message warned about: a citation rots when a
        // number frozen in one file outlives an edit to another, and a number
        // recomputed from the file it points into cannot. If the signature moves,
        // this moves with it; if it stops being unique, this reds rather than
        // silently matching a protector born somewhere else.
        let arm_source = read(&crate_root().join("src").join("task").join("scoped.rs"));
        let arm_hits = arm_source.lines().filter(|l| l.contains(ARM_SIGNATURE)).count();
        assert_eq!(
            arm_hits, 1,
            "`{ARM_SIGNATURE}` occurs {arm_hits} times in src/task/scoped.rs and must occur once. \
             It is how the receipt rule locates the arm's protector site; zero means the arm was \
             renamed or deleted, and more than one means the rule could accept a protector born at \
             the wrong site"
        );
        let arm_line = arm_source
            .lines()
            .position(|l| l.contains(ARM_SIGNATURE))
            .expect("invariant: the unique-occurrence assertion above just found it")
            + 1;
        let needle = format!("scoped.rs:{arm_line}:");
        assert!(
            text.contains(&needle),
            "{RECEIPT_DIR}/tb-neg-m2w-{seed}.stderr does not attribute a tag to `{needle}`, the \
             line src/task/scoped.rs currently puts `{ARM_SIGNATURE}` on. The receipt must pin the \
             protector to THE ARM's signature; the function name is not available to match on, \
             because Miri renders the `created here` help as a location with no source snippet"
        );
    }
}
