//! Compile-fail gate: a diagnostic code's **class** and its **number** are paired by the compiler.
//!
//! # What this gate exists to prevent, and why nothing else could catch it
//!
//! Until the macros took the typed newtype, `warn!`/`error!` accepted `$code:expr` into a `u16`
//! field while the **class byte came from the macro's own name**. Nothing joined the two, so
//!
//! ```ignore
//! warn!(Render, codes::E2103.number(), "…")   // compiled
//! ```
//!
//! emitted a `W`-class line carrying `2103` — a code `explain(b'W', 2103)` cannot resolve, and one
//! that no registry check could see, because **every one of them keys on the identifier in
//! source**: the orphan scan finds `E2103`, the doc-page check finds `E2103.md`, and check 5 finds
//! a test naming `E2103`. All of them are satisfied by a site that prints something else.
//!
//! It was found by reading a RED's output rather than by any gate. **Measured at the time: 62 of
//! 62 production invocations paired correctly**, so the hole was latent and not live — which is
//! exactly the state in which a hole is worth closing, and exactly the state in which nothing will
//! close it for you.
//!
//! Each `.rs` file in `tests/compile_fail_codes/` must fail to compile with the diagnostic recorded
//! in its matching `.stderr`. Regenerate the baseline — and **as standard procedure on toolchain
//! bumps**, since snapshot-based compile-fail tests are toolchain-coupled — via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-log --test compile_fail_codes
//! ```
//!
//! Covered cases:
//!
//! * `warn_rejects_an_error_code.rs` — `warn!` handed an `ErrorCode`.
//! * `error_rejects_a_warn_code.rs` — `error!` handed a `WarnCode`, the symmetric direction.
//! * `warn_rejects_a_bare_number.rs` — `warn!` handed `2103u16`. This one is the point: passing
//!   `.number()` is what every production site did, and what made the pairing decorative.

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_codes/*.rs");
}
