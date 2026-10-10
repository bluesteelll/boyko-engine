// EXPECT: items=2 blanket=1 test-only=
//
// Checker fixture for `scripts/check_hotpath_exceptions.py` — read by its self-test, compiled by
// no crate. See `cfg_all_test.rs` for the `EXPECT` contract.
//
// The negative twin: `any(test, ..)` does NOT require `test`. The item ships whenever the other
// arm is on — `boyko_rhi_vulkan`'s `#[cfg(any(test, feature = "goldens"))] mod goldens;` and
// `boyko_render`'s `any(test, feature = "test-readback")` are the in-tree cases — so every site
// below is still a production exception and the gate must still count or flag it. A `not(test)`
// site is production by definition.

pub struct Column;

#[cfg(any(test, feature = "test-readback"))]
#[allow(clippy::disallowed_types)]
static READBACK_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(not(test))]
#[allow(clippy::disallowed_types)]
static SHIPPING_ONLY: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Out-of-line declaration that ships under the feature: the sibling stays production.
#[cfg(any(test, feature = "goldens"))]
mod cfg_any_test_goldens;

#[cfg(any(test, feature = "goldens"))]
mod goldens {
    #![allow(clippy::disallowed_types)]

    use std::collections::HashMap;

    pub fn pins() -> HashMap<u32, u32> {
        HashMap::new()
    }
}
