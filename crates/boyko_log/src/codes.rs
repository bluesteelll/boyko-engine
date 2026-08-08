//! The diagnostic-code registry.
//!
//! One `codes! { … }` invocation generates a `pub const` per code, a **dense** table sorted by
//! number, a dense index per code, and [`explain`]. A `"boyko-…"` literal outside this registry is
//! a build failure — enforced by the registry walker, not by convention.
//!
//! # Class is a TYPE, not a field
//!
//! `warn!` takes a [`WarnCode`], `error!` takes an [`ErrorCode`], and [`PanicCode`] is a third
//! type. A class mismatch does not compile. A single `Code(u16)` with a `class: u8` field would
//! have made "warn with a panic code" a runtime concern, and a runtime concern about a
//! *diagnostic* is one nobody sees until the diagnostic matters.
//!
//! # `Live` / `Pending` / `Historical`, and why the grandfathered codes are all `Pending`
//!
//! Nine codes already exist in this engine's sources — as **string literals** in panic messages
//! and `#[should_panic]` expectations, never as identifiers. The registry's orphan check scans for
//! the *identifier* (`codes::B1802`), which does not appear anywhere until the migration rungs
//! rewrite those call sites. So:
//!
//! - [`CodeStatus::Live`] — the orphan check requires at least one identifier use, **and** a
//!   `docs/diagnostics/<code>.md` page. Register a `Live` code and emit it nowhere ⇒ red.
//! - [`CodeStatus::Pending`] — the premature-emitter check requires **zero** identifier uses. Emit
//!   a `Pending` code ⇒ red, which forces its row to flip to `Live` in the same commit that lands
//!   the emitter. A `Pending` row cannot rot silently: the day it acquires an emitter it reds.
//! - [`CodeStatus::Historical`] — zero emitters permitted, no page required, never becomes `Live`.
//!   For codes that exist only in frozen artifacts this repository will not edit.
//!
//! **All nine grandfathered codes are `Pending` at this rung**, which is what lets it land alone.
//! The registry corpus file says of `B9004`/`B9005` that "both are `Live` from L2 (they have
//! emitters today)"; that contradicts its own orphan rule, because *today's* occurrences are
//! literals and comments, not identifiers, and a `Live` row with no identifier use reds
//! immediately. The check semantics win. Their doc pages are written now anyway — a page for a
//! `Pending` row is permitted and the debt was already identified, so discharging it early costs
//! nothing and removes a thing to forget.

use core::sync::atomic::{AtomicBool, Ordering};

/// How often a `W`/`E` code may be delivered.
///
/// Declared **on the code**, applied **per site**. Declaring it on the code is what makes the
/// policy visible in the registry — a reader can see that `W2102` is `Once` without finding every
/// emitter — while applying it per site keeps two unrelated call sites from silencing each other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RatePolicy {
    /// Every occurrence.
    Every,
    /// The first occurrence at each site; later ones are dropped **without being counted**.
    ///
    /// The steady state is a pure `Relaxed` load of a site-local latch: no store, no shared line.
    Once,
    /// As [`Once`](RatePolicy::Once), but each suppressed occurrence pays one RMW so the count is
    /// real. A code whose suppressed count genuinely matters declares this and pays for it, at
    /// its own declaration site, with the cost written in the row.
    OnceCounted,
    /// One in `n`. **`n` must be a power of two**, so the test is `count & (n - 1)`.
    EveryN(u32),
    /// At most one per `ms` milliseconds.
    MinIntervalMs(u32),
}

/// Whether a row has emitters yet, and whether it ever will.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeStatus {
    /// Has emitters. Owes a doc page.
    Live,
    /// Reserved for a named rung; must have **no** emitters yet.
    Pending(&'static str),
    /// Exists only in frozen artifacts. No emitters, no page, never `Live`.
    Historical,
}

impl CodeStatus {
    /// Whether the orphan check requires an identifier use of this row.
    #[must_use]
    pub const fn requires_emitter(self) -> bool {
        matches!(self, CodeStatus::Live)
    }

    /// Whether the premature-emitter check requires **zero** identifier uses.
    #[must_use]
    pub const fn forbids_emitter(self) -> bool {
        matches!(self, CodeStatus::Pending(_) | CodeStatus::Historical)
    }
}

/// One registry row.
pub struct DiagInfo {
    /// The printed number, e.g. `1802` for `boyko-B1802`.
    pub number: u16,
    /// `b'B'`, `b'E'` or `b'W'`.
    pub class: u8,
    /// One line, imperative, no trailing period.
    pub summary: &'static str,
    /// Delivery policy.
    pub rate: RatePolicy,
    /// Migration state.
    pub status: CodeStatus,
}

/// A `W`-class code: a condition the caller probably did not intend.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WarnCode(u16);

/// An `E`-class code: the engine could not do what was asked.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ErrorCode(u16);

/// A `B`-class code: a broken invariant. Appears only inside a `#[cold] fn … -> !` or a `panic!`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PanicCode(u16);

macro_rules! code_newtype {
    ($T:ident, $class:expr) => {
        impl $T {
            /// The printed number.
            #[inline]
            #[must_use]
            pub const fn number(self) -> u16 {
                self.0
            }

            /// The class byte.
            #[inline]
            #[must_use]
            pub const fn class(self) -> u8 {
                $class
            }

            /// This code's dense registry index — the rate-table slot.
            ///
            /// Dense and equal to the row's position in the sorted table, so the rate array needs
            /// no map and no hash. Returns [`CODE_IDX_EXHAUSTED`] for a code the registry does not
            /// carry, which is unreachable for engine codes (they are minted by the macro) and is
            /// the downstream path's failure mode.
            #[inline]
            #[must_use]
            pub fn code_idx(self) -> u32 {
                code_idx_of($class, self.0)
            }
        }
    };
}

code_newtype!(WarnCode, b'W');
code_newtype!(ErrorCode, b'E');
code_newtype!(PanicCode, b'B');

/// The index a code gets when no rate slot could be assigned.
///
/// Engine codes are minted at compile time and can never see it. A downstream table that exhausts
/// the dynamic index space gets this, and a site holding it falls back to
/// [`RatePolicy::Every`] semantics with no rate state — a **degradation that is counted**, never a
/// panic and never a silent re-use of somebody else's slot.
pub const CODE_IDX_EXHAUSTED: u32 = u32::MAX;

/// Declare the registry.
///
/// Rows must be in **strictly increasing number order**, which is checked by a `const` assert:
/// a duplicate or an out-of-order row does not compile. Two rows may never share a number **even
/// across classes** — the table is dense with `index == code_idx`, so `W0110` and a hypothetical
/// `E0110` would collide in the rate array rather than merely read oddly.
macro_rules! codes {
    ($(
        ($num:expr, $class:ident, $Ident:ident, $rate:expr, $status:expr, $summary:literal)
    ),* $(,)?) => {
        $(
            #[doc = concat!("`boyko-", stringify!($class), stringify!($num), "` — ", $summary, ".")]
            pub const $Ident: code_class_ty!($class) = code_class_ty!($class)($num);
        )*

        /// Every row, sorted by number. `index == code_idx`.
        pub static DIAGNOSTICS: &[DiagInfo] = &[$(
            DiagInfo {
                number: $num,
                class: stringify!($class).as_bytes()[0],
                summary: $summary,
                rate: $rate,
                status: $status,
            },
        )*];

        const _: () = assert!(
            numbers_strictly_increasing(&[$($num),*]),
            "registry rows must be in strictly increasing number order: a duplicate, a \
             cross-class collision, or an out-of-order row"
        );
        const _: () = assert!(
            rate_policies_are_representable(&[$($rate),*]),
            "EveryN(n) requires n to be a power of two, so the test is `count & (n - 1)`: an \
             arbitrary n mis-samples across the counter wrap, invisibly in a bench and wrongly \
             in a session"
        );
    };
}

/// Maps a class token in [`codes!`] to its newtype.
///
/// `macro_rules!` is textually scoped, and what has to be in scope is the **invocation** site,
/// not the other macro's definition — both are defined above the single `codes! { … }` call at
/// the bottom of this file, which is where expansion happens. No `use` is needed and one was
/// removed: it was an unused import that `-D warnings` correctly refused.
macro_rules! code_class_ty {
    (B) => {
        PanicCode
    };
    (E) => {
        ErrorCode
    };
    (W) => {
        WarnCode
    };
}

/// `true` when every number is greater than the one before it.
const fn numbers_strictly_increasing(nums: &[u16]) -> bool {
    let mut i = 1;
    while i < nums.len() {
        if nums[i - 1] >= nums[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `true` when every `EveryN(n)` has a power-of-two `n`.
///
/// An arbitrary `n` forces `count % n`, which mis-samples across the `u32` counter wrap — about
/// twelve hours at 100 K·s⁻¹, so invisible in a three-hundred-frame bench and wrong in a session.
/// The power-of-two form is *also* cheaper: an `and` instead of a division. Strictly better on
/// both axes, which is why it is a compile error rather than a lint.
const fn rate_policies_are_representable(rates: &[RatePolicy]) -> bool {
    let mut i = 0;
    while i < rates.len() {
        if let RatePolicy::EveryN(n) = rates[i]
            && !n.is_power_of_two()
        {
            return false;
        }
        i += 1;
    }
    true
}

/// The dense index of `(class, number)`, or [`CODE_IDX_EXHAUSTED`].
///
/// Linear over a table of tens; called once per site at first use and cached in the site's own
/// cell, never per emission.
#[must_use]
pub fn code_idx_of(class: u8, number: u16) -> u32 {
    let mut i = 0;
    while i < DIAGNOSTICS.len() {
        if DIAGNOSTICS[i].class == class && DIAGNOSTICS[i].number == number {
            return i as u32;
        }
        i += 1;
    }
    CODE_IDX_EXHAUSTED
}

/// The registry row for `(class, number)`.
#[must_use]
pub fn explain(class: u8, number: u16) -> Option<&'static DiagInfo> {
    let idx = code_idx_of(class, number);
    if idx == CODE_IDX_EXHAUSTED { None } else { Some(&DIAGNOSTICS[idx as usize]) }
}

/// A per-site `Once` latch.
///
/// **Per site, not per code.** Two unrelated call sites sharing a code must not silence each
/// other, and the steady state must be a pure load: `Relaxed` on an `AtomicBool` that is already
/// `true` is one instruction and touches no line any other thread writes. A shared rate slot would
/// have put every `Once` site of one code on the same cache line, contended precisely during the
/// storm the policy exists to damp.
pub struct OnceSite {
    fired: AtomicBool,
}

impl OnceSite {
    /// A latch that has not fired.
    #[must_use]
    pub const fn new() -> OnceSite {
        OnceSite { fired: AtomicBool::new(false) }
    }

    /// `true` the first time only.
    ///
    /// The load short-circuits the exchange, so the steady state performs **no store**. The
    /// exchange is `Relaxed` on both sides: this latch orders nothing but itself, and a lost race
    /// between two threads at a site's very first occurrence delivers the record once, which is
    /// what `Once` promises.
    #[inline]
    pub fn claim(&self) -> bool {
        if self.fired.load(Ordering::Relaxed) {
            return false;
        }
        !self.fired.swap(true, Ordering::Relaxed)
    }

    /// Whether this site has fired. Read by the census walk, never on the hot path.
    #[must_use]
    pub fn has_fired(&self) -> bool {
        self.fired.load(Ordering::Relaxed)
    }
}

impl Default for OnceSite {
    fn default() -> Self {
        OnceSite::new()
    }
}

// ───────────────────────────────── the registry ──────────────────────────────────
//
// NINE grandfathered codes, measured against the tree rather than inherited: a scan for
// `boyko-[BEW][0-9]{4}` over `crates/**/*.rs` returns 89 occurrences and exactly these nine
// distinct codes. Every one is `Pending`, because today's occurrences are string literals and
// doc comments -- the orphan check scans IDENTIFIERS, and no identifier exists until the
// migration rungs rewrite the call sites.
//
// Every `summary` below is taken from the message the engine ACTUALLY prints, read out of the
// source, not written from the code's name. The first draft of this table invented three of them
// -- B0002 as "a system parameter the world cannot supply" (it is an intra-system access conflict
// on one resource), B9002 as "conflicting access to one resource" (it is a cycle in the SET
// hierarchy) and B9004 as "a set that was never registered" (it is two ordered sets sharing a
// member). A registry summary that disagrees with the panic text sends a reader looking for the
// wrong thing, and nothing downstream would ever have flagged it.
//
// `B9003` is a PERMANENT GAP. It exists in `docs/archive/**` only; the archive is outside the
// corpus and a code that promises no future emitter must not be seeded as `Pending`, which is a
// promise. The gap is recorded here so the next reader does not "fix" the sequence.
//
// EIGHTEEN `92xx` rows are reserved for the profiler, consecutive and with no gaps, so the code
// space is claimed before that subsystem's rungs need it. Measured first: the `9xxx` band is
// already occupied by `B9001`/`B9002`/`B9004`/`B9005`/`B9101`, but `92xx` itself is free in
// source -- zero `92xx` literals under `crates/` or `src/`.
codes! {
    (2,    B, B0002, RatePolicy::Every, CodeStatus::Pending("L6"),
        "Intra-system access conflict on one resource"),
    // The FIRST `Live` row in this registry. Its summary is read from the emitter
    // (`sink/file.rs::render_cap`) rather than composed from the code's name -- which is the rule
    // three rows of this registry's first draft broke.
    (103,  W, W0103, RatePolicy::Once,  CodeStatus::Live,
        "The file sink reached its byte cap and stopped writing"),
    (1501, W, W1501, RatePolicy::Once,  CodeStatus::Pending("L6"),
        "Ordering references a system set that has no members"),
    (1801, B, B1801, RatePolicy::Every, CodeStatus::Pending("L8b"),
        "A plugin was added more than once"),
    (1802, B, B1802, RatePolicy::Every, CodeStatus::Pending("L8b"),
        "An App method was called after finish(), in the run phase"),
    (9001, B, B9001, RatePolicy::Every, CodeStatus::Pending("L6"),
        "The schedule contains a cycle of systems"),
    (9002, B, B9002, RatePolicy::Every, CodeStatus::Pending("L6"),
        "The set hierarchy contains a cycle of sets"),
    (9004, B, B9004, RatePolicy::Every, CodeStatus::Pending("L6"),
        "Two ordered sets share a member, so a system would run both before and after itself"),
    (9005, B, B9005, RatePolicy::Every, CodeStatus::Pending("L6"),
        "Ordering references a system key that is not in this schedule"),
    (9101, B, B9101, RatePolicy::Every, CodeStatus::Pending("L6"),
        "Schedule::run was called with a different world than the one it was built on"),

    (9201, W, W9201, RatePolicy::Once,  CodeStatus::Pending("profiling 1"),
        "A profiling zone was opened on a thread holding no lane"),
    (9202, W, W9202, RatePolicy::Once,  CodeStatus::Pending("profiling 1"),
        "A profiling zone was closed that was never opened"),
    (9203, W, W9203, RatePolicy::Once,  CodeStatus::Pending("profiling 1"),
        "A profiling lane overflowed and samples were discarded"),
    // Class `E`, not `W`, and the difference is load-bearing rather than cosmetic: the profiling
    // corpus writes this one as the literal `boyko-E9204`, and check 1 forbids two rows sharing a
    // number EVEN ACROSS CLASSES, because the table is dense with `index == code_idx`. A `W9204`
    // row beside that literal would be both undeclared (check 4) and un-addable (check 1). The
    // ladder's shorthand says "W9201..W9218"; the literal that actually exists wins.
    (9204, E, E9204, RatePolicy::Once,  CodeStatus::Pending("profiling 5"),
        "The profiler was bound to a second world"),
    (9205, W, W9205, RatePolicy::Once,  CodeStatus::Pending("profiling 2"),
        "A profiling fold observed a clock epoch break and quarantined its window"),
    (9206, W, W9206, RatePolicy::Once,  CodeStatus::Pending("profiling 2"),
        "A profiling window closed with no samples in it"),
    (9207, W, W9207, RatePolicy::Once,  CodeStatus::Pending("profiling 4"),
        "A GPU timestamp query pool returned fewer results than were issued"),
    (9208, W, W9208, RatePolicy::Once,  CodeStatus::Pending("profiling 4"),
        "A GPU timestamp was rejected because the device reported a zero period"),
    (9209, W, W9209, RatePolicy::Once,  CodeStatus::Pending("profiling 4"),
        "A GPU zone was closed on a different command buffer than it was opened on"),
    (9210, W, W9210, RatePolicy::Once,  CodeStatus::Pending("profiling 10"),
        "A dynamic zone name was truncated to fit the name arena"),
    (9211, W, W9211, RatePolicy::Once,  CodeStatus::Pending("profiling 10"),
        "The declared user zone budget exceeds the profile's recommended maximum"),
    (9212, W, W9212, RatePolicy::Once,  CodeStatus::Pending("profiling 10"),
        "The dynamic zone registry is full and further zones are unnamed"),
    (9213, W, W9213, RatePolicy::Once,  CodeStatus::Pending("profiling 11"),
        "A retention tier dropped a window before it was read"),
    (9214, W, W9214, RatePolicy::Once,  CodeStatus::Pending("profiling 12"),
        "A telemetry window was written while the previous write had not completed"),
    (9215, W, W9215, RatePolicy::Once,  CodeStatus::Pending("profiling 12"),
        "A telemetry destination refused a write and the window was dropped"),
    (9216, W, W9216, RatePolicy::Once,  CodeStatus::Pending("profiling 3"),
        "A statistics query was asked for a band it has no samples for"),
    (9217, W, W9217, RatePolicy::Once,  CodeStatus::Pending("profiling 3"),
        "A contrast query compared two windows taken under different configurations"),
    (9218, W, W9218, RatePolicy::Once,  CodeStatus::Pending("profiling 13"),
        "A profiling scope entity was despawned while its zone was still open"),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_strictly_increasing_rejects_what_it_must() {
        // The real table's check is a compile error, so the checker itself is exercised on the
        // inputs the table must never contain.
        assert!(numbers_strictly_increasing(&[]));
        assert!(numbers_strictly_increasing(&[2, 1501, 9218]));
        assert!(!numbers_strictly_increasing(&[2, 2]), "a duplicate must be rejected");
        assert!(!numbers_strictly_increasing(&[1501, 2]), "out of order must be rejected");
    }

    #[test]
    fn rate_policy_checker_rejects_a_non_power_of_two() {
        assert!(rate_policies_are_representable(&[RatePolicy::EveryN(1)]));
        assert!(rate_policies_are_representable(&[RatePolicy::EveryN(1024), RatePolicy::Once]));
        assert!(!rate_policies_are_representable(&[RatePolicy::EveryN(3)]));
        assert!(!rate_policies_are_representable(&[RatePolicy::EveryN(0)]), "0 is not a power of two");
    }

    #[test]
    fn the_table_is_dense_and_index_equals_code_idx() {
        // The rate array is indexed by `code_idx` with no map and no hash, so this equality IS
        // the addressing scheme. If it ever stops holding, every rate lookup reads another code's
        // slot -- silently, and only under load.
        for (i, row) in DIAGNOSTICS.iter().enumerate() {
            assert_eq!(
                code_idx_of(row.class, row.number),
                i as u32,
                "row {i} (class {}, number {}) does not resolve to its own index",
                row.class as char,
                row.number
            );
        }
    }

    #[test]
    fn every_pending_row_names_its_rung_and_the_live_set_is_pinned() {
        // At L2 this asserted that EVERY row was `Pending` -- the property that let that rung land
        // alone. L4 lands the first `Live` row, so the claim moves rather than loosens: a Pending
        // row must still name its rung, and the Live set is enumerated here so a row that goes
        // Live without its emitter, or an emitter that lands without flipping its row, reds.
        const LIVE: &[(u8, u16)] = &[(b'W', 103)];
        for row in DIAGNOSTICS {
            match row.status {
                CodeStatus::Pending(rung) => {
                    assert!(!rung.is_empty(), "a Pending row must name the rung that lands it");
                    assert!(
                        !LIVE.contains(&(row.class, row.number)),
                        "row {}{} is in the pinned Live set but its status is Pending",
                        row.class as char,
                        row.number
                    );
                }
                CodeStatus::Live => assert!(
                    LIVE.contains(&(row.class, row.number)),
                    "row {}{} went Live without being added to this test's pinned set",
                    row.class as char,
                    row.number
                ),
                other => panic!(
                    "row {}{} is {other:?}; only Pending and Live exist at this rung",
                    row.class as char, row.number
                ),
            }
        }
    }

    #[test]
    fn the_nine_grandfathered_codes_are_all_present() {
        // Measured against the tree: `boyko-[BEW][0-9]{4}` over `crates/**/*.rs` returns exactly
        // these nine distinct codes. Pinned so a later edit cannot quietly drop one -- a dropped
        // row makes its literal undeclared, which the walker's check 4 would then red on.
        for (class, number) in
            [(b'B', 2), (b'W', 1501), (b'B', 1801), (b'B', 1802), (b'B', 9001), (b'B', 9002),
             (b'B', 9004), (b'B', 9005), (b'B', 9101)]
        {
            assert!(
                explain(class, number).is_some(),
                "grandfathered code {}{number:04} is missing from the registry",
                class as char
            );
        }
    }

    #[test]
    fn b9003_is_a_permanent_gap() {
        // It exists only in `docs/archive/**`, which is outside the corpus. Seeding it as
        // `Pending` would promise a future emitter that will never arrive. Asserted so that a
        // later reader "completing the sequence" fails a test instead of shipping a lie.
        assert!(explain(b'B', 9003).is_none());
    }

    #[test]
    fn the_profiling_block_is_eighteen_consecutive_rows() {
        let block: Vec<u16> =
            DIAGNOSTICS.iter().filter(|r| (9201..=9299).contains(&r.number)).map(|r| r.number).collect();
        assert_eq!(block.len(), 18, "the 92xx reservation is eighteen rows");
        for (i, n) in block.iter().enumerate() {
            assert_eq!(*n, 9201 + i as u16, "the block must be consecutive with no gaps");
        }
    }

    #[test]
    fn classes_are_distinct_types_so_a_mismatch_cannot_compile() {
        // The runtime half of a compile-time property: the numbers agree with the newtype, which
        // is what makes `warn!(…, B1802)` a type error rather than a wrong line in a log.
        assert_eq!(B1802.number(), 1802);
        assert_eq!(B1802.class(), b'B');
        assert_eq!(W1501.class(), b'W');
        assert_eq!(W9201.class(), b'W');
    }

    #[test]
    fn explain_resolves_a_known_code_and_refuses_an_unknown_one() {
        let info = explain(b'W', 1501).expect("W1501 is registered");
        assert_eq!(info.rate, RatePolicy::Once);
        assert!(!info.summary.is_empty());
        assert!(explain(b'W', 1802).is_none(), "1802 is a B code; the class must be part of the key");
        assert!(explain(b'E', 9999).is_none());
        assert_eq!(code_idx_of(b'E', 9999), CODE_IDX_EXHAUSTED);
    }

    #[test]
    fn a_once_latch_fires_exactly_once_and_then_only_loads() {
        let l = OnceSite::new();
        assert!(!l.has_fired());
        assert!(l.claim(), "the first claim wins");
        assert!(!l.claim(), "every later claim loses");
        assert!(!l.claim());
        assert!(l.has_fired());
    }

    #[test]
    fn concurrent_first_claims_deliver_exactly_one() {
        // `Once` promises delivery once, not delivery on a particular thread. A latch that let
        // two racing threads both win would double-log the first occurrence of every code the
        // engine reports from parallel systems.
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        let latch = Arc::new(OnceSite::new());
        let wins = Arc::new(AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let (l, w) = (Arc::clone(&latch), Arc::clone(&wins));
            hs.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    if l.claim() {
                        w.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in hs {
            h.join().expect("claimant thread panicked");
        }
        assert_eq!(wins.load(Ordering::SeqCst), 1, "exactly one claim may win");
    }
}
