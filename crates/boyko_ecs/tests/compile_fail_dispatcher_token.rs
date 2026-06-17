//! Phase 5 Option C — `compile_fail` acceptance tests for the `DispatcherToken`
//! soundness rework.
//!
//! Two load-bearing properties are proven by *failure to compile*:
//!
//! * **M1** (`token_double_mut_aliases_rejected.rs`) — two
//!   `DispatcherToken::nonsend_resource_mut` calls with BOTH results held live
//!   must NOT compile. The `&mut self`-tied return lifetime makes a second
//!   `&mut R` un-aliasable; if it compiled, the M1 aliasing hole would be open.
//! * **C1** (`unsafe_cell_nonsend_accessor_deleted.rs`) — code calling the
//!   now-DELETED `UnsafeEcsCell::nonsend_resource_mut` must NOT compile. This
//!   proves the worker-reachable `!Send` projection surface is gone (the C1
//!   kill); if the method still existed the case would compile.
//!
//! Each `.rs` file in `tests/compile_fail_dispatcher_token/` is expected to fail
//! to compile with the diagnostic recorded in its matching `.stderr` file.
//! Regenerate the baseline when revising a case via:
//!
//! ```powershell
//! $env:TRYBUILD = "overwrite"
//! cargo test -p boyko-ecs --test compile_fail_dispatcher_token
//! ```

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail_dispatcher_token/*.rs");
}
