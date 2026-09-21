// SIBLING-OF: cfg_any_test.rs
// Sibling of `cfg_any_test.rs`, declared there as `#[cfg(any(test, feature = "goldens"))] mod
// cfg_any_test_goldens;`. The `goldens` feature compiles it into the library, so it is production
// and must NOT be classified test-only — which is what `cfg_any_test.rs`'s empty `test-only=`
// expectation asserts.
#![allow(clippy::disallowed_types)]

use std::collections::HashMap;

pub fn pins() -> HashMap<u32, u32> {
    HashMap::new()
}
