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
//! `#![cfg(not(miri))]` like every trybuild harness in the tree: the harness shells out to
//! `cargo`, which Miri cannot execute.
//!
//! [`NestedCursor`]: boyko_reflect::cursor::NestedCursor
#![cfg(not(miri))]

/// The corpus. Both cases must FAIL to compile, with the blessed message.
#[test]
fn a_cursor_cannot_outlive_or_alias_the_value_it_reads() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/c6_compile_fail/*.rs");
}
