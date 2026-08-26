//! CORE C6 gate 5 — **the `'a` on [`NestedCursor`] is compiler-enforced, and this is the
//! only instrument that can say so.**
//!
//! Every other gate in this campaign asserts what a program *does*. This one asserts what
//! a program **cannot be**: a cursor held across a `&mut` to the value it reads, and a
//! cursor outliving that value. Analysis M2/O3 says the bare `{ptr, info}` cursor is
//! *"deleted and never introduced"* — with no lifetime, both fixtures below compile and
//! the use-after-free becomes a runtime question. C6's second RED deletes the `'a` and the
//! `_pd` to watch exactly that happen.
//!
//! # Why `--no-fail-fast` matters for this target specifically
//!
//! A `.stderr` corpus pins compiler prose. MEASURED in this repository: a trybuild fixture
//! stayed red for **87 commits** because a line was added and its `.stderr` was never
//! re-blessed — invisible because `cargo test` stops at the first failing target. The
//! bytes here are additionally covered by the repository-wide compiler freeze in
//! `tests/trybuild_corpus_compiler_witness.rs`; re-bless only with a stated reason.
//!
//! # The floor, added 2026-08-26, because this harness had none
//!
//! **An empty glob is a VACUOUS PASS**: `trybuild` prints *"There are no trybuild tests
//! enabled yet"*, the harness reports `running 1 test … ok`, and the process exits **0**.
//! `running N` cannot see it — the harness function runs and passes over zero fixtures. As
//! shipped, this file had no floor at all, so **deleting both fixtures left C6 gate 5 green
//! while asserting nothing**, and the same emptiness could arrive by a one-character slip in
//! the glob rather than by a deletion. `crates/boyko_reflect/tests/seam_census.rs` calls this
//! file out by name as the vacuous shape an implementer inherits by proximity; it was true.
//!
//! The floor counts [`CORPUS`], which is also what the glob is built from — a floor over a
//! directory the harness does not compile guards nothing, and that decoupling was measured on
//! two other harnesses in this tree the same day. `tests/trybuild_corpus_compiler_witness.rs`
//! gates the class: every `trybuild` glob in the repository must resolve to a fixture.
//!
//! `#![cfg(not(miri))]` like every trybuild harness in the tree: the harness shells out to
//! `cargo`, which Miri cannot execute.
//!
//! [`NestedCursor`]: boyko_reflect::cursor::NestedCursor
#![cfg(not(miri))]

/// The corpus directory, relative to `crates/boyko_reflect/tests` — the **one** spelling the
/// floor counts and the glob expands.
const CORPUS: &str = "c6_compile_fail";

/// How many `.rs` fixtures the corpus actually holds. A missing directory is a **failure**,
/// never a skip: a harness that cannot find its corpus looks exactly like one that scanned it.
fn fixture_count() -> usize {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(CORPUS);
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("the C6 corpus must exist at {}: {e}", dir.display()))
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .count()
}

/// The corpus. Both cases must FAIL to compile, with the blessed message.
#[test]
fn a_cursor_cannot_outlive_or_alias_the_value_it_reads() {
    let n = fixture_count();
    assert!(
        n >= 2,
        "the C6 corpus holds {n} fixture(s) and C6 gate 5 lands TWO -- a cursor held across a \
         `&mut`, and a cursor outliving the value it reads. `>=`, so adding a fixture never reds \
         this; below 2 the harness would run on an emptier corpus than the gate claims, and an \
         empty one is a VACUOUS PASS at exit 0 that `running N` cannot see."
    );
    let t = trybuild::TestCases::new();
    t.compile_fail(format!("tests/{CORPUS}/*.rs"));
}
