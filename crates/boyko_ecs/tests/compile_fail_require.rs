//! Feature 1 (required components) — `compile_fail` acceptance tests for the
//! `#[require(...)]` derive attribute.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` Resolved open questions: a duplicate
//! same-id `#[require(B, B)]` is a COMPILE error (the macro sees both keys —
//! strictly better than Bevy's runtime panic); an empty `#[require()]` is
//! rejected.
//!
//! Each `.rs` file under `tests/compile_fail_require/` is compiled in isolation;
//! the matching `.stderr` baseline records the expected compiler diagnostic.
//! Regenerate the baselines after a rustc / syn point release that shifts
//! diagnostic wording via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_require
//! ```
//!
//! KE11 (ballot AB-6 arm (a)) adds the storage-kind refusal: `#[require(<a
//! bitset flag>)]` is a COMPILE error, because `RequiredCtor` exists only to
//! write BYTES into an uninitialized storage slot and a flag has none. The
//! derive cannot resolve the required path to a `StorageKind` — it holds a
//! token, not a type — so it emits a `const _: () = assert!(!<Ty as
//! Component>::STORAGE_IS_BITSET, …)` item and lets the COMPILER answer, at
//! `cargo check` time, at the one place that knows. `storage = "dense"` is
//! deliberately NOT refused (it has bytes; only its store differs), and
//! `require_bitset_flag_rejected.rs` carries a dense entry in the same
//! `#[require(...)]` list as the standing control on that: trybuild matches the
//! `.stderr` exactly, so a refusal that over-fired onto dense could not pass.
//!
//! | File                                     | Rejected input                          |
//! |------------------------------------------|-----------------------------------------|
//! | `duplicate_require_rejected.rs`          | `#[require(B, B)]` (same id twice)      |
//! | `empty_require_rejected.rs`               | `#[require()]` (no entries)             |
//! | `require_bitset_flag_rejected.rs`         | `#[require(<bitset flag>)]`, bare path  |
//! | `require_bitset_flag_with_ctor_rejected.rs` | `#[require(<bitset flag> = expr)]`    |
//!
//! ⚠ The two KE11 baselines quote the fixture's own source LINE, so editing a
//! fixture `.rs` above its `#[require(...)]` shifts the span and must be
//! re-blessed — the baselines are not merely message goldens.
//!
//! Gated behind `#[cfg(not(miri))]` — trybuild is not wired under Miri (mirrors
//! `compile_fail_hooks.rs`).

#![cfg(not(miri))]

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_require/*.rs");
}
