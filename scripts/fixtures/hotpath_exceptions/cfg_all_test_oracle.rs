// SIBLING-OF: cfg_all_test.rs
// Sibling of `cfg_all_test.rs`, declared there as `#[cfg(all(test, not(loom)))] mod
// cfg_all_test_oracle;`. Never compiled into a library, so its crate-level `#![allow]` is not a
// blanket suppression — provided the checker resolves the declaration, which is what
// `cfg_all_test.rs`'s `test-only=` expectation asserts.
#![allow(clippy::disallowed_types)]

use std::collections::HashMap;

pub fn model() -> HashMap<u32, u32> {
    HashMap::new()
}
