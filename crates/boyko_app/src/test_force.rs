//! Render-wave plan H0 Scope 8: **`TestForce`**, the test-only force-key set a fixture uses to hold
//! a regime the production predicates would not choose on their own (RD1's `disarm_sdf_merge`,
//! PX1a's `inject_nan_lit`, R3's `no_push_descriptor`, R6's `cov_off`, R7's `oit_probe`, …).
//!
//! # Why the environment decode is not in this crate's shipping code
//!
//! The precedent this follows, [`crate::occlusion_force`], records what the boot-time
//! `std::env::var` with a boot panic in it cost and why it was moved OUT to the fixtures that need
//! it. So this module holds only the type — a `Resource`, absent in production, which a consumer
//! reads through `try_resource` and treats as the empty set — and a pure, allocation-free
//! [`parse`]. A fixture that needs a key reads `BOYKO_TEST_FORCE` itself, calls [`parse`] and
//! inserts the resource at boot; an unknown key is the named [`UnknownTestForceKey`] error, which
//! the fixture turns into its own boot refusal.
//!
//! # Keys
//!
//! Each rung that needs a key adds its variant, its spelling in [`TestForceKey::ALL`] and its
//! consumer's plumbing in one commit:
//!
//! - `disarm_sdf_merge` (render wave RD1, defect R0b) — [`TestForceKey::DisarmSdfMerge`].
//! - `srgb_swapchain` (render wave PX1a, D-P1) — [`TestForceKey::SrgbSwapchain`].

use boyko_macros::Resource;

/// One test-only force key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestForceKey {
    /// Render wave RD1 (defect R0b): removes the SDF depth merge — the red control of G18 and the
    /// mesh-depth gate, and the disarmed arm of the WR cost probe. On VisibilityBuffer × Sdf the
    /// merge is `vb_depth`'s first writer, so there it stays declared as a CLEAR-only pass (the
    /// pre-RD1 regime with a defined depth); every other path declares no merge. The renderer's
    /// transparent-depth order guard is silent under it, by name.
    DisarmSdfMerge,
    /// Render wave PX1a (D-P1, test 17): the window host picks an 8-bit `*_SRGB` swapchain
    /// (`SurfaceFormatPick::ForceSrgb`) — and refuses the boot when the surface advertises none —
    /// so the present blit's `srgb_eotf` variant is the one that runs. Read once, at boot, by the
    /// windowed runner; the shipping pick prefers `B8G8R8A8_UNORM`.
    SrgbSwapchain,
}

impl TestForceKey {
    /// Every key with its `BOYKO_TEST_FORCE` spelling; a key's bit is its index here.
    pub const ALL: &'static [(TestForceKey, &'static str)] = &[
        (TestForceKey::DisarmSdfMerge, "disarm_sdf_merge"),
        (TestForceKey::SrgbSwapchain, "srgb_swapchain"),
    ];

    /// This key's bit in [`TestForce`].
    #[inline]
    pub const fn bit(self) -> u64 {
        match self {
            TestForceKey::DisarmSdfMerge => 1 << 0,
            TestForceKey::SrgbSwapchain => 1 << 1,
        }
    }
}

/// The set of force keys a fixture armed at boot. `Default` is the empty set, which is also what a
/// consumer assumes when the resource is absent (every production run).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TestForce {
    bits: u64,
}

impl TestForce {
    /// The empty set.
    pub const EMPTY: Self = Self { bits: 0 };

    /// `true` iff `key` is armed.
    #[inline]
    pub const fn contains(self, key: TestForceKey) -> bool {
        self.bits & key.bit() != 0
    }

    /// `true` iff no key is armed.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }
}

/// A `BOYKO_TEST_FORCE` item that names no [`TestForceKey`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UnknownTestForceKey<'a> {
    /// The unknown key, trimmed, borrowed from the parsed value.
    pub key: &'a str,
}

impl core::fmt::Display for UnknownTestForceKey<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "BOYKO_TEST_FORCE: unknown key '{}' (known keys: ", self.key)?;
        for (i, (_, name)) in TestForceKey::ALL.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(name)?;
        }
        f.write_str(")")
    }
}

/// Parses a `<key>[,<key>…]` value: items are comma-separated, whitespace around an item is
/// ignored and empty items are skipped, so `""` and `" , "` are the empty set. Allocates nothing.
///
/// # Errors
///
/// The first item that names no key, as [`UnknownTestForceKey`].
pub fn parse(value: &str) -> Result<TestForce, UnknownTestForceKey<'_>> {
    let mut bits = 0u64;
    for item in value.split(',') {
        let key = item.trim();
        if key.is_empty() {
            continue;
        }
        match TestForceKey::ALL.iter().find(|(_, name)| *name == key) {
            Some(&(k, _)) => bits |= k.bit(),
            None => return Err(UnknownTestForceKey { key }),
        }
    }
    Ok(TestForce { bits })
}

#[cfg(test)]
mod tests {
    use super::{parse, TestForce, TestForceKey, UnknownTestForceKey};

    #[test]
    fn every_key_round_trips_through_its_spelling() {
        for (i, &(key, name)) in TestForceKey::ALL.iter().enumerate() {
            assert_eq!(key.bit(), 1 << i, "a key's bit is its index in ALL ({name})");
            let force = parse(name).unwrap_or_else(|e| panic!("{e}"));
            assert!(force.contains(key), "`{name}` parses to a set containing its key");
            assert!(!force.is_empty());
        }
        let disarm = parse(" disarm_sdf_merge ,").expect("RD1's key parses with separators");
        assert!(disarm.contains(TestForceKey::DisarmSdfMerge));
        assert!(!TestForce::EMPTY.contains(TestForceKey::DisarmSdfMerge));
    }

    #[test]
    fn empty_string_is_the_empty_set() {
        assert_eq!(parse(""), Ok(TestForce::EMPTY));
        assert!(TestForce::default().is_empty());
    }

    #[test]
    fn unknown_key_is_a_named_error() {
        let err = parse("disarm_everything").expect_err("an unknown key must be refused");
        assert_eq!(err, UnknownTestForceKey { key: "disarm_everything" });
        let text = err.to_string();
        assert!(text.contains("unknown key 'disarm_everything'"), "{text}");
    }

    #[test]
    fn separators_and_whitespace() {
        assert_eq!(parse(" , ,,"), Ok(TestForce::EMPTY));
        assert_eq!(parse("  bogus_key  ,"), Err(UnknownTestForceKey { key: "bogus_key" }));
        assert_eq!(parse(",,second , bogus"), Err(UnknownTestForceKey { key: "second" }));
    }
}
