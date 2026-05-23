//! Compile-fail tests for the `#[event]` attribute macro.
//!
//! Each `.rs` file in `tests/ui/event_attribute/` is expected to fail to
//! compile, with the exact `compile_error!` diagnostic recorded in a
//! matching `.stderr` file (run `TRYBUILD=overwrite cargo test --test
//! event_attribute_ui` to regenerate the `.stderr` files when adding
//! new cases).

#[cfg(not(miri))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/event_attribute/*.rs");
}
