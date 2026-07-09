//! Task #9 — `compile_fail` acceptance tests for the `Option<D>` / `AnyOf<(..)>`
//! compile-time gates (spec `docs/OPTION-ANYOF-PLAN.md` Decisions 3, 5, 7 +
//! the read-only `iter()` reject).
//!
//! Each `.rs` file under `tests/option_anyof_compile_fail/` is compiled in
//! isolation; the matching `.stderr` baseline records the expected compiler
//! diagnostic. Regenerate the baselines after a rustc point release that
//! shifts diagnostic wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo +stable-x86_64-pc-windows-gnu test -p boyko-ecs --test option_anyof_compile_fail
//! ```
//!
//! # Covered cases
//!
//! | File                                  | Gate                                                  |
//! |---------------------------------------|-------------------------------------------------------|
//! | `iter_rejects_option_mut.rs`          | `iter()` rejects `Option<&mut T>` (ReadOnlyQueryData) |
//! | `iter_rejects_anyof_mut.rs`           | `iter()` rejects `AnyOf<(&mut A,)>`                    |
//! | `anyof_arm_option_rejected.rs`        | `AnyOf<(&A, Option<&B>)>` — Option is not AnyOfArm    |
//! | `anyof_arm_unit_rejected.rs`          | `AnyOf<((), &A)>` — `()` is not AnyOfArm              |
//! | `anyof_empty_rejected.rs`             | empty `AnyOf<()>` — no impl                            |
//! | `for_each_chunk_rejects_option.rs`    | `for_each_chunk` rejects `Option<&T>` (no ChunkedQD)  |
//! | `for_each_chunk_rejects_anyof.rs`     | `for_each_chunk` rejects `AnyOf<..>` (no ChunkedQD)   |
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild's driver is not wired under
//! Miri (mirrors `compile_fail_chunk.rs`).

#![cfg(not(miri))]

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/option_anyof_compile_fail/*.rs");
}
