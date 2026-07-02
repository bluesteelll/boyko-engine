//! R0b — `compile_fail` acceptance tests for [`FrameWriteToken`]'s AFFINE move
//! semantics (`!Clone`, `!Copy`, private construction).
//!
//! Three load-bearing properties are proven by *failure to compile*:
//!
//! * **use-after-move** (`token_use_after_submit_rejected.rs`) — a token passed
//!   BY VALUE to the frame-ending `Renderer::render_gbuffer_frame` must NOT be
//!   usable afterwards: the submit ends the frame's host-write window, so a
//!   per-slot write proof surviving the submit would reopen the write-after-read
//!   ring hazard the token exists to close.
//! * **no Clone** (`token_clone_rejected.rs`) — `token.clone()` must NOT
//!   compile. A clonable proof would let the caller keep writing after the
//!   consume (or stash a token across frames), voiding the affine discipline.
//! * **no forging via literal** (`token_literal_construction_rejected.rs`) —
//!   `FrameWriteToken { slot: 0 }` from outside the crate must NOT compile (the
//!   field is private). The ONLY mints are `Renderer::wait_frame_in_flight`
//!   (the fence proof) and the audited `unsafe` setup hatch
//!   `FrameWriteToken::forge_unfenced`.
//!
//! Each `.rs` file in `tests/compile_fail_frame_write_token/` is expected to
//! fail to compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the baseline when revising a case via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko_rhi_vulkan --test compile_fail_frame_write_token
//! ```

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_frame_write_token/*.rs");
}
