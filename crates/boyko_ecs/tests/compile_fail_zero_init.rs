//! **Profiling rung 10, `G22b` second clause — the storage policy's own RED, as a compile error.**
//!
//! # What the corpus asks for, and the two corrections measured on the way
//!
//! `G22b`'s second clause reads: *"a `#[test]` declaring a `.bss` array sized from a
//! `ProfilerConfig` value must fail `assert_bss_eligible` **at compile time**; remove the
//! const-assert ⇒ it compiles ⇒ red."* Two things in that sentence do not survive contact with the
//! tree:
//!
//! **1. `assert_bss_eligible` does not exist.** MEASURED: zero hits anywhere under `crates/`. The
//! symbol is `boyko_diag::storage::assert_zero_init_eligible`, and it gates a different property —
//! not "is this extent a compile-time constant" but "is `T`'s all-zero bit pattern a valid `T`".
//!
//! **2. The failure the clause describes is structurally impossible here, which is stronger than a
//! gate.** `SyncCells<T, N>` takes its extent as a **const generic**. A `ProfilerConfig` field is a
//! run-time value, so `SyncCells<T, cfg.user_zone_budget>` does not compile whatever any assertion
//! says — the type system enforces the policy's first half with no help. "Remove the const-assert
//! ⇒ it compiles" is therefore false for this implementation: there is no const-assert to remove,
//! because there is nothing for one to catch.
//!
//! # What this file gates instead — the property rung 10 actually introduced
//!
//! The policy's *other* half is real and newly load-bearing. `ZoneDesc` carries a `&'static str`,
//! and a null reference is not a valid `&str` — so `ZoneDesc` can never be `ZeroInit` and
//! `DYN_DESCS` had to be a `SyncCells<MaybeUninit<ZoneDesc>, _>`. That wrapper is easy to read as
//! ceremony and delete. If it is deleted, the arena stops being zero-initialisable, and the honest
//! outcome is a compile error rather than a static that quietly leaves `.bss` and carries raw data
//! in the image.
//!
//! The case below is exactly that deletion. It must not compile.
//!
//! ```text
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_zero_init
//! ```

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_zero_init/*.rs");
}
