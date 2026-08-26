//! **CORE C9 / GATES G5 — the derive's refusal corpus: one trybuild harness, three globs,
//! two legs.**
//!
//! Every rejection `#[component(reflect)]` makes is a `compile_error!` spanned at the
//! offending token, and every one of them has a fixture here whose blessed `.stderr` is
//! the message's bytes. `tests/reflect_refusal_census.rs` asserts the bijection between
//! this directory and `boyko_macros`'s `REFUSALS` table, in both directions.
//!
//! # Why this package and not `boyko_ecs` (D33, measured in both directions)
//!
//! C9 as first written named `crates/boyko_ecs/tests/compile_fail_*.rs`, the established
//! harness. That package **cannot host this corpus**: it declares no `reflect` feature,
//! trybuild copies the host manifest's `[features]` table into the generated crate, and
//! the derive's entire reflect emission is `#[cfg(feature = "reflect")]` evaluated
//! *there*. Every fixture would compile and the harness would red with *"expected compile
//! failure"* on all of them — a gate that cannot go green, whose repair by re-blessing
//! would be vacuity. MEASURED: `#[derive(Component)] #[component(reflect)]` on a struct in
//! `boyko_ecs`'s own test tree compiles clean (which is itself the proof the `cfg` is dead
//! there — `boyko_ecs` has no `boyko-reflect` edge, so a live `cfg` would be `E0433`) and
//! emits `warning: unexpected cfg condition value: reflect`, which CI's clippy promotes
//! under `-D warnings`.
//!
//! # The two legs, and why the second is what makes this test the DERIVE
//!
//! * **Feature ON** — every census fixture fails with its blessed `.stderr`.
//! * **Feature OFF** — every census fixture **compiles**. D33 puts each `compile_error!`
//!   inside the same `#[cfg(feature = "reflect")]` block as the emission it guards, so a
//!   feature-off consumer gets a program that compiles to nothing rather than a refusal of
//!   a program that compiles to nothing. A corpus that reds identically in both legs would
//!   be testing rustc.
//!
//! # Three globs, because one of them cannot satisfy both legs
//!
//! `reflect_compile_fail_upstream/` holds the two fixtures whose refusal is **not C9's**:
//! a generic component and a `#[repr(packed)]` one. D34 struck both rows from `REFUSALS`
//! because `#[derive(Component)]` and rustc already refuse those inputs — *a row in
//! `REFUSALS` that C9 does not author is a fixture whose red cannot fire*, since deleting
//! C9's refusal leaves the program non-compiling anyway. They are kept as **regression
//! pins**: the day the derive threads generics, or stops taking `&field`, the obligation
//! returns and these two `.stderr` files are what says so. They are deliberately outside
//! the census directory (they pin rustc's and `#[derive(Component)]`'s diagnostics, not
//! C9's) and they run in the feature-ON leg only, because their output DIFFERS between
//! legs — a generic fixture is 15 errors with the feature off and 20 with it on — and one
//! fixture cannot carry two blessed files.
//!
//! ⚠️ That asymmetry is also a finding about G5 rather than about this file. G5's second
//! leg says *"Feature off: every fixture **compiles**"* and its second RED expects *"the
//! harness reds on all nine at once"*; both are **false today for two of its nine
//! fixtures**, before C9 lands anything, and §7.3d records it.
//!
//! # `reflect_pass/` is RUN, not merely compiled
//!
//! trybuild's `check_pass` builds the fixture **and executes the binary**, requiring
//! success (verified in trybuild 1.0.120). So a `t.pass()` fixture can assert a runtime
//! property, and the five here do: the `#[reflect(skip)]` way out of the `Opaque`-field
//! row, D20's `#[reflect(no_default)]` opt-out, the dense-storage positive control, C8's
//! migrated bitset clauses, and the ACCEPTANCE half of the `#[repr(Int)]` enum rule —
//! whose refusal half was pinned while `has_integer_repr` returning **true** was reached
//! by nothing (MEASURED: forcing it to `false` left every gate green).
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test reflect_compile_fail --no-fail-fast
//! cargo test -p reflect-fixture --test reflect_compile_fail --no-fail-fast
//! ```
//!
//! `--no-fail-fast` because `cargo test` stops at the first failing target, so one
//! known-red target shadows every target behind it — this repository has measured a
//! trybuild fixture staying red for **87 commits** because a line was added and its
//! `.stderr` was never re-blessed, invisible until the flag was passed. It is an
//! *invocation*, not an instrument: nothing in the tree reds if it is omitted, which is
//! why it is listed here and not counted as one of C9's gates.
//!
//! `#![cfg(not(miri))]` like every trybuild harness in the tree: the harness shells out to
//! `cargo`, which Miri cannot execute. The bytes these fixtures pin are additionally
//! covered by the repository-wide compiler freeze in
//! `tests/trybuild_corpus_compiler_witness.rs`; re-bless only with a stated reason, and
//! **never** without first confirming `rustc --version` — a shadowing standalone
//! `rustc 1.95.0` can bless a corpus the mandated 1.97.1 then rejects.
#![cfg(not(miri))]

/// **An empty glob is a VACUOUS PASS, measured.** `trybuild` prints *"There are no
/// trybuild tests enabled yet"* and returns success, so every clause in this file would
/// stay green with its subject directory emptied.
///
/// The census (`tests/reflect_refusal_census.rs`) covers `reflect_compile_fail/` by
/// bijection with `REFUSALS`, but nothing outside this file knows how many fixtures the
/// other two directories are supposed to have. So each glob states its own floor, as a
/// `>=` — adding a fixture must never red a count, and deleting the last one must.
fn fixture_count(dir: &str) -> usize {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(dir);
    std::fs::read_dir(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .count()
}

/// **Leg 1 (feature ON).** Every refusal C9 authors, with its blessed message.
#[cfg(feature = "reflect")]
#[test]
fn every_refusal_fails_with_its_blessed_message() {
    let n = fixture_count("reflect_compile_fail");
    assert!(n >= 6, "the refusal corpus holds {n} fixtures, and C9 lands 6");
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/reflect_compile_fail/*.rs");
}

/// **Leg 1, the upstream pins.** Not C9's refusals — see the header.
#[cfg(feature = "reflect")]
#[test]
fn the_two_upstream_refusals_still_refuse() {
    let n = fixture_count("reflect_compile_fail_upstream");
    assert!(n >= 2, "the upstream pins are {n}, and D34 struck exactly 2 rows");
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/reflect_compile_fail_upstream/*.rs");
}

/// **Leg 1, the accepting twins.** Compiled *and run* — see the header on `check_pass`.
#[cfg(feature = "reflect")]
#[test]
fn every_accepted_shape_compiles_and_runs() {
    let n = fixture_count("reflect_pass");
    assert!(n >= 5, "the accepting corpus holds {n}: C9's 4, plus the `#[repr(Int)]` twin");
    let t = trybuild::TestCases::new();
    t.pass("tests/reflect_pass/*.rs");
}

/// **Leg 2 (feature OFF).** The SAME directory, and now every fixture must **compile**.
///
/// This is the leg that makes the corpus a test of the derive rather than of rustc: a
/// refusal that fired here would be refusing a program whose reflect emission does not
/// exist. It is also the leg that would have caught D33 being got wrong, which is why it
/// is a test rather than a paragraph.
#[cfg(not(feature = "reflect"))]
#[test]
fn the_same_fixtures_compile_with_the_feature_off() {
    let n = fixture_count("reflect_compile_fail");
    assert!(n >= 6, "the refusal corpus holds {n} fixtures, and C9 lands 6");
    let t = trybuild::TestCases::new();
    t.pass("tests/reflect_compile_fail/*.rs");
}
