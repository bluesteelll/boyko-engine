//! L6 — `boyko-B0502` is the terminal panic when the query-type table runs out.
//!
//! Its own integration binary, because filling the process-global mint counter past the cap is a
//! one-way transition: any sibling test that later asked for a `QueryTypeId` would panic for this
//! test's reason rather than its own. See `l6_query_table_high_water.rs`, which burns 768 slots
//! for the same reason at one remove.
//!
//! **This is also the gate on `PanicCode`'s `Display`.** L6 replaced the string literal
//! `"boyko-B0502: …"` with the registry constant so the identifier reaches the walker's CODE
//! stream; the `expected` substring below is what proves the rendered text did not move with it.
//! Delete the `Display` impl's `boyko-` prefix, or write the code as an inline `{B0502}` format
//! argument, and this reds.

use boyko_ecs::ecs::core::iters::query::query_type_registry::{MAX_QUERY_TYPES, register_new};

#[test]
#[should_panic(expected = "boyko-B0502")]
fn b0502_is_the_terminal_panic_when_the_table_is_exhausted() {
    // One past the cap. `register_new` saturates the counter before panicking, so the loop cannot
    // run it further even if the panic were caught.
    for _ in 0..=MAX_QUERY_TYPES {
        let _ = register_new();
    }
}
