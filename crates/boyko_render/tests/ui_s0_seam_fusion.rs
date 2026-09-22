//! UI-ADVANCED rung S0 gate G0-5 — the SEAM GATE's call-site half:
//! **re-fusing the two-phase upload seam must fail to compile.**
//!
//! Phase 1 of [`boyko_render::UiUploadSystem`]'s `run_dispatcher` reads the
//! world through the token's read-only `WorldView` (`&self` of the token);
//! Phase 2 projects the `!Send` `RhiContext` through `nonsend_resource_mut`
//! (`&mut self` of the token). A body holding BOTH at one call site — the
//! shape of the deleted `host_upload_frame_from_world`, whose parameter list
//! demanded exactly that — is the M1 borrow conflict (`dispatcher_token.rs`:
//! "a `WorldView` cannot coexist with `nonsend_resource_mut`"), and the
//! fixture pins the compiler refusing it with E0502. That makes the fusion
//! UNREPRESENTABLE rather than merely fixed: the defect class cannot be
//! reintroduced without this suite going red.
//!
//! Each `.rs` file in `tests/ui_s0_seam_fusion/` is expected to fail to
//! compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the baseline when revising a case via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-render --test ui_s0_seam_fusion
//! ```
//!
//! (Under the mandated rustup toolchain — the `.stderr` corpus is coupled to
//! the compiler; see `tests/trybuild_corpus_compiler_witness.rs` at the repo
//! root.)

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui_s0_seam_fusion/*.rs");
}
