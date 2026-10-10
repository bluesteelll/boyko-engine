// EXPECT: items=0 blanket=0 test-only=cfg_all_test_oracle.rs
//
// Checker fixture for `scripts/check_hotpath_exceptions.py` — read by its self-test, compiled by
// no crate. The `EXPECT` line above is the assertion: how many per-item exceptions and blanket
// `#![allow]` sites the checker must report for this file, and which out-of-line module it must
// classify as test-only.
//
// Every site below sits under a cfg predicate that REQUIRES `test` — `all(test, ..)` is false
// whenever `test` is false, whichever position `test` takes — so none of them exists in a shipping
// build and none is a production exception. This is the exact shape that held the gate RED at
// `crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs:544` (ff64c6be): the checker matched
// only the bare `#[cfg(test)]` spelling, so a `#[cfg(all(test, not(loom)))] mod tests` read as
// production and its `#![allow]` as an unregistered blanket suppression.

pub struct Reservoir;

#[cfg(all(test, not(loom)))]
#[allow(clippy::disallowed_types)]
static ORACLE_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `test` in the second position, behind another guard.
#[allow(clippy::disallowed_types)]
#[cfg(all(not(miri), test))]
static ORACLE_MODEL: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

/// Out-of-line declaration: the sibling file is pulled in only under `test`.
#[cfg(all(test, not(loom)))]
mod cfg_all_test_oracle;

#[cfg(all(test, target_arch = "x86_64", target_feature = "avx2"))]
mod simd_oracle {
    #![allow(clippy::disallowed_types)]

    use std::collections::HashMap;

    pub fn model() -> HashMap<u32, u32> {
        HashMap::new()
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    // Test-only oracle state — the reference the reservoir is checked against, never engine data.
    #![allow(clippy::disallowed_types)]

    use super::*;
}
