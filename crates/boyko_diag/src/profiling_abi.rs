//! Layer B — the profiling ABI, **hosted** here, not shared.
//!
//! # Hosted, and the word is load-bearing
//!
//! This crate's growth rule admits a thing only if **both** subsystems write it and a disagreement
//! between two copies would show up in a joined artifact. `profiling_abi` fails that test: the
//! profiler writes it and `boyko_log` never names it. It is here for a **graph** reason instead.
//!
//! The ABI must sit below `boyko_threadpool` and `boyko_rhi_vulkan`, because both open zones.
//! Before this crate the only thing below everything was `boyko_utils`, which must keep an empty
//! `[dependencies]` and must not become a second one. So the ABI is *hosted* by the bottom crate
//! and namespaced to say so — it is `profiling_abi`, not `abi`, and this paragraph is why.
//!
//! **`boyko_log` must never name anything in this module.** That is a mechanical check, not a
//! convention: a grep for `ZoneTier`, `ARM_MASK` or `declare_zone` in the logger's sources must
//! return nothing.
//!
//! # The two-axis gate
//!
//! ```text
//! const { $handle::TIER as u8 <= GLOBAL_TIER as u8 }   // (a) compile: folds, deletes codegen
//!     && ARM_MASK.load(Acquire) & scope_bit != 0       // (b) runtime: one Acquire load, one bt
//! ```
//!
//! **The tier is read from the `mod` companion, never through the handle static**, and that is not
//! a style choice: the handle carries an `AtomicU16`, and a `const` block that reads through it is
//! `error[E0080]: constant accesses mutable global memory`. The obvious spelling does not compile.
//! [`declare_zone!`] therefore emits **two** items per zone — a `static` in the value namespace and
//! a `mod` in the type namespace, sharing one name — and the gate reads the module's `const`.
//!
//! # What the two axes buy, and what neither does
//!
//! Axis (a) is the **compile ceiling**: `const false` short-circuits the `&&` and the arm and its
//! operands vanish. Axis (b) is the **runtime flag** and is the site's floor — one `Acquire` load
//! of a cache-padded global plus one statically-predicted-not-taken branch, at every surviving
//! site, forever. A flag has to be read in order to be a flag, so disarming cannot drive (b) to
//! zero. Only the tier removes the site.
//!
//! **The tier fold deletes CODEGEN, not TOKENS.** The expansion names its handle twice — once in
//! the gate's `const` block and once in the guard body — and name resolution runs on both whichever
//! way the const folds. A mistyped zone identifier is therefore `E0425` in **every** profile,
//! retail included. Only the feature axis, which deletes the macro definition before name
//! resolution, can hide one.

use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};

/// How much a build is willing to compile in.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ZoneTier {
    /// Survives every profile, including retail. For zones a shipped title still wants.
    Always = 0,
    /// Development builds.
    Dev = 1,
    /// Deep instrumentation: per-item, per-draw, per-entity.
    Deep = 2,
}

impl ZoneTier {
    /// Build a tier from its raw discriminant.
    ///
    /// # Panics
    ///
    /// On a value outside `0..=2`. Used only from a `const` context, where that is a **compile
    /// error** — a build script that emitted a nonsense tier must stop the build, not silently
    /// instrument the wrong amount of a shipped title.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u8) -> ZoneTier {
        match raw {
            0 => ZoneTier::Always,
            1 => ZoneTier::Dev,
            2 => ZoneTier::Deep,
            _ => panic!("invariant: a tier is 0..=2; see boyko_diag::profile::PROFILING_TIER"),
        }
    }
}

/// The compile-time tier ceiling for this build. A zone above it does not exist.
pub const GLOBAL_TIER: ZoneTier = ZoneTier::from_raw(crate::profile::PROFILING_TIER);

/// Number of scopes a build can arm independently.
pub const SCOPE_COUNT: u32 = 64;

/// The runtime flag word: bit *s* set means scope *s* is armed.
///
/// **Its own cache line.** It is read by every surviving site on every frame and written only when
/// a scope is armed or disarmed, so sharing a line with anything that changes would invalidate it
/// for every reader on every write. `.bss`-zero means disarmed, which is what makes a process that
/// never arms the profiler free of it without an initialiser.
#[repr(C, align(64))]
struct ArmMask {
    bits: AtomicU64,
    _pad: [u8; 56],
}

static ARM_MASK: ArmMask = ArmMask { bits: AtomicU64::new(0), _pad: [0; 56] };

const _: () = assert!(core::mem::size_of::<ArmMask>() == 64);
const _: () = assert!(core::mem::align_of::<ArmMask>() == 64);

/// Immutable per-zone metadata, one `'static` per [`declare_zone!`].
pub struct ZoneDesc {
    /// Printed name.
    pub name: &'static str,
    /// Which scope's bit arms this zone.
    pub scope: u32,
    /// Declared tier. Duplicated into the `mod` companion, which is what the gate reads —
    /// see the module docs on why the gate cannot read it from here.
    pub tier: ZoneTier,
}

/// A declared zone: its metadata and the id the registry assigns on first use.
pub struct ZoneHandle {
    /// Cold metadata.
    pub desc: &'static ZoneDesc,
    /// Registry id, minted once. **This field is why the gate cannot read the tier through the
    /// handle**: an `AtomicU16` makes the whole static mutable global memory, and a `const` block
    /// that reads through it is `E0080`.
    pub id: AtomicU16,
}

impl ZoneHandle {
    /// Declare a handle. `const`, so the static costs no initialiser.
    #[must_use]
    pub const fn new(desc: &'static ZoneDesc) -> ZoneHandle {
        ZoneHandle { desc, id: AtomicU16::new(0) }
    }
}

/// Whether scope `s` is armed. One `Acquire` load and one bit test.
///
/// `Acquire` rather than `Relaxed`: arming publishes the buffers a sample will be written into, so
/// a site that observes the bit must observe those too.
#[inline]
#[must_use]
pub fn scope_armed(scope: u32) -> bool {
    debug_assert!(scope < SCOPE_COUNT, "invariant: a scope index is below SCOPE_COUNT");
    ARM_MASK.bits.load(Ordering::Acquire) & (1u64 << (scope % SCOPE_COUNT)) != 0
}

/// Arm a scope. Runs on the enable path, never at process start.
pub fn arm_scope(scope: u32) {
    ARM_MASK.bits.fetch_or(1u64 << (scope % SCOPE_COUNT), Ordering::Release);
}

/// Disarm a scope.
pub fn disarm_scope(scope: u32) {
    ARM_MASK.bits.fetch_and(!(1u64 << (scope % SCOPE_COUNT)), Ordering::Release);
}

/// Whether anything at all is armed — the one load a caller needs to skip a whole subsystem.
#[inline]
#[must_use]
pub fn any_armed() -> bool {
    ARM_MASK.bits.load(Ordering::Acquire) != 0
}

/// Declare a zone: **two items sharing one name**, in two namespaces.
///
/// ```ignore
/// declare_zone!(VB_EARLY_RASTER, name = "vb.early_raster", scope = 3, tier = ZoneTier::Dev);
/// ```
///
/// expands to a `static VB_EARLY_RASTER: ZoneHandle` (value namespace) **and** a
/// `mod VB_EARLY_RASTER { pub const TIER: ZoneTier = …; }` (type namespace). A `static` and a `mod`
/// may share a name; a `struct` and a `static` may not, which is why the companion is a module and
/// not a marker type.
///
/// **The companion exists because the gate cannot read the tier from the handle.** The handle
/// carries an `AtomicU16`, so a `const` block reading through it is `E0080: constant accesses
/// mutable global memory`. Measured, not assumed — the obvious spelling was specified for four
/// revisions and does not compile.
///
/// The `use super::*;` inside the module is also load-bearing: a macro-emitted `mod` is a fresh
/// scope that inherits none of the caller's imports, so without it `ZoneTier` is unresolvable at
/// every expansion site that did not happen to glob-import it.
#[macro_export]
macro_rules! declare_zone {
    ($ident:ident, name = $name:literal, scope = $scope:expr, tier = $tier:expr $(,)?) => {
        #[doc = concat!("Zone `", $name, "`.")]
        pub static $ident: $crate::profiling_abi::ZoneHandle =
            $crate::profiling_abi::ZoneHandle::new(&$crate::profiling_abi::ZoneDesc {
                name: $name,
                scope: $scope,
                tier: $tier,
            });

        #[doc = concat!("Compile-time facts about zone `", $name, "`.")]
        #[allow(non_snake_case)]
        pub mod $ident {
            use super::*;
            /// The declared tier, readable from a `const` block — which the handle static is not.
            pub const TIER: $crate::profiling_abi::ZoneTier = $tier;
            /// The arming scope.
            pub const SCOPE: u32 = $scope;
        }
    };
}

/// The gate a zone site expands to.
///
/// Written as a macro rather than a function so gate (a) is a `const` block the compiler folds and
/// short-circuits: a function call would evaluate its arguments, which is the entire property the
/// `&&` chain exists to prevent.
#[macro_export]
macro_rules! zone_enabled {
    ($handle:ident) => {
        const { $handle::TIER as u8 <= $crate::profiling_abi::GLOBAL_TIER as u8 }
            && $crate::profiling_abi::scope_armed($handle::SCOPE)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    declare_zone!(TEST_ALWAYS, name = "t.always", scope = 1, tier = ZoneTier::Always);
    declare_zone!(TEST_DEEP, name = "t.deep", scope = 2, tier = ZoneTier::Deep);

    #[test]
    fn the_handle_and_the_companion_share_a_name_in_two_namespaces() {
        // The two-item expansion, exercised through BOTH namespaces at once. A `struct` companion
        // would be `E0428` here; a `mod` is not.
        assert_eq!(TEST_ALWAYS.desc.name, "t.always");
        assert_eq!(TEST_ALWAYS::TIER, ZoneTier::Always);
        assert_eq!(TEST_ALWAYS::SCOPE, 1);
        assert_eq!(TEST_ALWAYS.desc.tier, TEST_ALWAYS::TIER, "the two copies must agree");
    }

    /// THE reason the companion exists — and the proof is that this **compiles**, not that any
    /// assertion in it passes.
    ///
    /// Reading `TEST_ALWAYS.desc.tier` in this `const` block instead would be `E0080: constant
    /// accesses mutable global memory`, because the handle carries an `AtomicU16`. So the const
    /// block IS the test; an `assert!(OK)` beneath it would be `assert!(true)`, which clippy
    /// correctly refuses and which would teach a reader that the assertions here are decoration.
    const _COMPANION_IS_CONST_READABLE: bool = TEST_ALWAYS::TIER as u8 <= GLOBAL_TIER as u8;

    #[test]
    fn a_disarmed_scope_gates_everything_out() {
        // `.bss`-zero is disarmed, and that is what makes a process that never arms free of the
        // profiler without an initialiser.
        disarm_scope(1);
        assert!(!scope_armed(1));
        assert!(!zone_enabled!(TEST_ALWAYS));
    }

    #[test]
    fn arming_one_scope_does_not_arm_its_neighbours() {
        disarm_scope(10);
        disarm_scope(11);
        arm_scope(10);
        assert!(scope_armed(10));
        assert!(!scope_armed(11), "scopes must be independent bits, not a single flag");
        disarm_scope(10);
        assert!(!scope_armed(10));
    }

    #[test]
    fn the_tier_gate_folds_independently_of_the_runtime_flag() {
        // Arm the scope as wide as it goes, so the ONLY thing that can refuse a site is its tier.
        // In the `dev` profile `GLOBAL_TIER` is `Deep`, so both survive; the assertion that
        // matters is the SHAPE -- a site above the ceiling is refused with the flag fully armed.
        arm_scope(2);
        assert!(scope_armed(2));
        // The handle static is referenced here on purpose. `zone_enabled!` reads ONLY the module
        // companion, so at this rung nothing touches the static at all -- clippy said so, and it
        // is right: the second naming of the identifier lives in the guard body, which arrives
        // with the sample path. Until then this is where the static is proved to exist and to
        // carry the same tier the gate reads.
        assert_eq!(TEST_DEEP.desc.tier, TEST_DEEP::TIER);
        assert_eq!(TEST_DEEP.desc.scope, TEST_DEEP::SCOPE);
        assert!(zone_enabled!(TEST_DEEP), "the dev profile's Deep ceiling admits a Deep zone");
        disarm_scope(2);
        assert!(!zone_enabled!(TEST_DEEP), "disarming must refuse regardless of tier");
    }

    #[test]
    fn any_armed_is_the_one_load_that_skips_a_subsystem() {
        disarm_scope(20);
        let quiet = !any_armed();
        arm_scope(20);
        assert!(any_armed());
        disarm_scope(20);
        assert_eq!(!any_armed(), quiet || !any_armed());
    }

    #[test]
    fn the_arm_mask_owns_its_cache_line() {
        // Asserted at run time as well as in the `const` asserts: a reordering that preserved the
        // size would pass those alone, and the whole point is that no neighbour shares the line a
        // hot reader loads every frame.
        assert_eq!(core::mem::size_of::<ArmMask>(), 64);
        assert_eq!(std::ptr::from_ref(&ARM_MASK) as usize % 64, 0);
    }
}
