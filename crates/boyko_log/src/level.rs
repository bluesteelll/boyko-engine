//! Severity, and the one property the whole design rests on: **`Off` is zero.**

/// Severity. Lower value = more severe.
///
/// `Off` is a **threshold value only** — no record is ever emitted at `Off`. That is what makes
/// `.bss`-zero mean "every target disabled": the control array is zeroed by the loader, so a
/// process that has not enabled logging is correctly and freely silent, with no initialiser run
/// and no page touched.
///
/// There is no `Fatal`. A fatal condition panics through a `#[cold] fn … -> !` helper and the
/// panic hook flushes; two spellings of "die" is one too many.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Level {
    /// Emits nothing. The `.bss`-zero state, and the meaning of an un-enabled target.
    Off = 0,
    /// The engine could not do what was asked and the caller must handle it.
    Error = 1,
    /// The engine did something the caller probably did not intend.
    Warn = 2,
    /// Lifecycle facts a developer wants without asking.
    Info = 3,
    /// Detail a developer asks for while working on a subsystem.
    Debug = 4,
    /// Per-item detail; expected to be expensive and expected to be off.
    Trace = 5,
}

impl Level {
    /// The most verbose level, and therefore the widest ceiling.
    pub const MAX: Level = Level::Trace;

    /// Number of bits a level occupies in a packed control byte. `Trace = 5` needs three.
    pub const BITS: u32 = 3;

    /// Build a level from its raw discriminant.
    ///
    /// # Panics
    ///
    /// In a `const` context an out-of-range value is a **compile error**, which is the only
    /// context this is used from: [`crate::GLOBAL_CEILING`] maps the substrate's raw `u8` through
    /// here. Keeping it fallible-by-panic rather than returning `Option` is deliberate — an
    /// invalid build-profile ceiling must stop the build, not degrade to a default that logs the
    /// wrong amount in a shipped title.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u8) -> Level {
        match raw {
            0 => Level::Off,
            1 => Level::Error,
            2 => Level::Warn,
            3 => Level::Info,
            4 => Level::Debug,
            5 => Level::Trace,
            _ => panic!("invariant: a level is 0..=5; see boyko_diag::profile::LOG_CEILING"),
        }
    }

    /// The lower-case name used in artifacts and in control specs.
    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        }
    }
}

// `Off` being 0 is not a stylistic choice: `.bss` is zeroed, so it is what makes an un-enabled
// process silent without an initialiser. Asserted rather than commented, because the day someone
// reorders this enum is the day a fresh process starts emitting at `Error` before boot.
const _: () = assert!(Level::Off as u8 == 0);
// The packing in `TargetControl` gives the level three bits. A sixth variant is fine; a ninth is
// not, and would silently alias into the sample-shift field rather than failing to compile.
const _: () = assert!((Level::MAX as u8) < (1 << Level::BITS));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_severity_ascending_so_a_ceiling_compare_reads_naturally() {
        // The gate is `ceiling >= level`. That only means "a wider ceiling admits more" if the
        // discriminants ascend with verbosity.
        assert!(Level::Off < Level::Error);
        assert!(Level::Error < Level::Warn);
        assert!(Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert!(Level::Debug < Level::Trace);
    }

    #[test]
    fn from_raw_round_trips_every_variant() {
        for raw in 0u8..=5 {
            assert_eq!(Level::from_raw(raw) as u8, raw);
        }
    }

    #[test]
    fn an_off_ceiling_admits_nothing_including_error() {
        // The property `.bss`-zero relies on, stated as a test rather than as a comment.
        for lvl in [Level::Error, Level::Warn, Level::Info, Level::Debug, Level::Trace] {
            assert!(
                (Level::Off as u8) < (lvl as u8),
                "an Off ceiling must refuse {lvl:?}"
            );
        }
    }
}
