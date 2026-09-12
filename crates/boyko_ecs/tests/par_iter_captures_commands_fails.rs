//! Phase 9 PAR1 / CQ-SEND2 — trybuild compile-fail harness for the
//! `par_iter` body capture rules.
//!
//! `Query::par_iter().for_each(closure)` requires
//! `closure: Fn(D::Item<'_>) + Send + Sync` (PAR1). `Commands<'s>` is `!Sync`
//! — since EM2′ through its `EntityCounter`'s `Cell` (the EXHAUSTED bit); the
//! `&'s mut CommandQueue` field alone never made it so, because `CommandQueue`
//! is auto-`Sync`. Capturing `&mut Commands` inside the `for_each` closure
//! therefore must fail to compile, and now fails at the `Sync` bound.
//!
//! The trybuild test files live in `tests/par_iter_compile_fail/`. Each
//! `.rs` file is compiled in isolation; its `.stderr` baseline is the
//! expected compiler error. When the language's diagnostic wording shifts
//! (rustc point release), update the baseline by running
//! `TRYBUILD=overwrite cargo test --test par_iter_captures_commands_fails`.
//!
//! Gated behind `#[cfg(not(miri))]` because Miri does not have the
//! trybuild driver wired and the test would not compile under Miri's
//! restricted env.

#![cfg(not(miri))]

#[test]
fn par_iter_captures_commands_must_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/par_iter_compile_fail/*.rs");
}
